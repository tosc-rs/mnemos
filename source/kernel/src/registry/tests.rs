use core::sync::atomic::{AtomicU32, Ordering};

use super::*;
use crate::{comms, test_util::TestKernel, Kernel};

struct TestService;

impl RegisteredDriver for TestService {
    type Request = TestMessage;
    type Response = TestMessage;
    type Error = TestMessage;
    type Hello = TestMessage;
    type ConnectError = TestMessage;
    const UUID: Uuid = uuid!("05bcd4b7-dd81-434a-a958-f18ee84f8635");
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct TestMessage(usize);

#[test]
fn konly_connect() {
    TestKernel::run(|k| async move {
        let (listener, registration) = listener::Listener::<TestService>::new(2).await;

        // server
        k.spawn(async move {
            loop {
                let conn = listener.handshake().await;
                if conn.hello == TestMessage(1) {
                    let (tx, rx) = crate::comms::kchannel::KChannel::new_async(2).await.split();
                    k.spawn(async move {
                        while let Ok(Message { msg, reply }) = rx.dequeue_async().await {
                            reply
                                .reply_konly(msg.reply_with_body(|TestMessage(val)| {
                                    tracing::info!(val, "received request");
                                    Ok(TestMessage(val + 1))
                                }))
                                .await
                                .unwrap();
                        }
                    })
                    .await;
                    conn.accept(tx).unwrap();
                } else {
                    conn.reject(TestMessage(666)).unwrap();
                }
            }
        })
        .await;

        k.with_registry(|r| r.register_konly(registration))
            .await
            .unwrap();

        let reply = comms::oneshot::Reusable::new_async().await;
        let mut client1 = k
            .registry()
            .await
            .connect_with_hello::<TestService>(TestMessage(1))
            .await
            .expect("connect should succeed");

        let rsp = client1
            .request_oneshot(TestMessage(1), &reply)
            .await
            .expect("client 1 request 1 should succeed");
        assert_eq!(rsp.body, Ok(TestMessage(2)));

        // should be rejected
        let res = k
            .registry()
            .await
            .connect_with_hello::<TestService>(TestMessage(2))
            .await;
        match res {
            Ok(_) => panic!("rejected connect should fail"),
            Err(ConnectError::Rejected(TestMessage(666))) => {}
            Err(e) => panic!(
                "rejected connect should return ConnectError::Rejected, got {:?}",
                e
            ),
        }

        let mut client2 = k
            .registry()
            .await
            .connect_with_hello::<TestService>(TestMessage(1))
            .await
            .expect("connect with accepted Hello should succeed");

        let rsp = client2
            .request_oneshot(TestMessage(2), &reply)
            .await
            .expect("client 2 request 1 should succeed");
        assert_eq!(rsp.body, Ok(TestMessage(3)));

        let rsp = client1
            .request_oneshot(TestMessage(3), &reply)
            .await
            .expect("client 1 request 2 should succeed");
        assert_eq!(rsp.body, Ok(TestMessage(4)));
    })
}

#[test]
fn user_connect() {
    TestKernel::run(|k| async move {
        let (listener, registration) = listener::Listener::<TestService>::new(2).await;

        // server
        k.spawn(async move {
            loop {
                let conn = listener.handshake().await;
                if conn.hello == TestMessage(1) {
                    let (tx, rx) = crate::comms::kchannel::KChannel::new_async(2).await.split();
                    k.spawn(async move {
                        while let Ok(Message { msg, reply }) = rx.dequeue_async().await {
                            let TestMessage(val) = msg.body;
                            tracing::info!(val, "received request");
                            match reply {
                                ReplyTo::Userspace { nonce, outgoing } => {
                                    let reply = UserResponse {
                                        nonce,
                                        uuid: TestService::UUID,
                                        reply: Ok::<_, TestMessage>(TestMessage(val + 1)),
                                    };

                                    let bytes =
                                        postcard::to_stdvec(&reply).expect("must serialize!");
                                    let mut wgr = outgoing.send_grant_exact(bytes.len()).await;
                                    wgr.copy_from_slice(&bytes[..]);
                                    wgr.commit(bytes.len());
                                }

                                _ => panic!("all requests should be from 'userspace'"),
                            }
                        }
                    })
                    .await;
                    conn.accept(tx).unwrap();
                } else {
                    conn.reject(TestMessage(666)).unwrap();
                }
            }
        })
        .await;

        k.with_registry(|r| r.register(registration)).await.unwrap();

        #[tracing::instrument(skip(k), err(Debug))]
        async fn user_connect(
            k: &'static Kernel,
            hello: TestMessage,
        ) -> Result<UserspaceHandle, UserConnectError<TestService>> {
            let bytes = postcard::to_stdvec(&hello).expect("must serialize!");

            k.registry()
                .await
                .connect_userspace_with_hello::<TestService>(&k.inner().scheduler, &bytes[..])
                .await
        }

        #[tracing::instrument(skip(user_tx, user_rx, handle), ret)]
        async fn user_request(
            handle: &UserspaceHandle,
            user_tx: &bbq::MpscProducer,
            user_rx: &bbq::Consumer,
            req: TestMessage,
        ) -> Result<TestMessage, TestMessage> {
            static NEXT_NONCE: AtomicU32 = AtomicU32::new(0);
            tracing::info!(?req, "sending user request");
            let bytes = postcard::to_stdvec(&req).expect("request must serialize");
            let nonce = NEXT_NONCE.fetch_add(1, Ordering::Relaxed);
            let user_req = UserRequest {
                req_bytes: &bytes[..],
                nonce,
                uid: TestService::UUID,
            };
            handle
                .process_msg(user_req, user_tx)
                .expect("process_msg should succeed");
            let rgr = user_rx.read_grant().await;
            let len = rgr.len();
            let rsp: UserResponse<TestMessage, TestMessage> =
                postcard::from_bytes(&rgr[..]).expect("response should deserialize");
            rgr.release(len);
            assert_eq!(rsp.nonce, nonce);
            rsp.reply
        }

        let (user_tx, user_rx) = bbq::new_spsc_channel(256).await;
        let user_tx = user_tx.into_mpmc_producer().await;

        let client1 = user_connect(k, TestMessage(1))
            .await
            .expect("connect should succeed");

        let rsp = user_request(&client1, &user_tx, &user_rx, TestMessage(1)).await;
        assert_eq!(Ok(TestMessage(2)), rsp);

        // should be rejected
        let res = user_connect(k, TestMessage(2)).await;
        match res {
            Ok(_) => panic!("request with rejected hello should fail"),
            Err(e) => assert_eq!(
                e,
                UserConnectError::Connect(ConnectError::Rejected(TestMessage(666)))
            ),
        };

        let client2 = user_connect(k, TestMessage(1))
            .await
            .expect("connect should succeed");

        let rsp = user_request(&client2, &user_tx, &user_rx, TestMessage(2)).await;
        assert_eq!(Ok(TestMessage(3)), rsp);

        let rsp = user_request(&client1, &user_tx, &user_rx, TestMessage(3)).await;
        assert_eq!(Ok(TestMessage(4)), rsp);
    })
}
