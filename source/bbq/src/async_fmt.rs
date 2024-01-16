use super::{BidiHandle, GrantW, MpscProducer, SpscProducer};
use core::fmt::{self, Write};

// TODO(eliza): should this be a public API?
pub(crate) trait Producer {
    async fn send_grant(&self, max: usize) -> GrantW;
}

impl Producer for MpscProducer {
    async fn send_grant(&self, max: usize) -> GrantW {
        self.send_grant_max(max).await
    }
}

impl Producer for SpscProducer {
    async fn send_grant(&self, max: usize) -> GrantW {
        self.send_grant_max(max).await
    }
}

impl Producer for BidiHandle {
    async fn send_grant(&self, max: usize) -> GrantW {
        self.producer().send_grant_max(max).await
    }
}

pub(crate) async fn fmt_to_bbq(producer: &impl Producer, fmt: impl fmt::Debug) {
    struct ByteCtr {
        cnt: usize,
    }

    impl fmt::Write for ByteCtr {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            self.cnt += s.as_bytes().len();
            Ok(())
        }
    }

    struct WriteWgr<'a> {
        skip: usize,
        written: usize,
        buf: &'a mut [u8],
    }

    impl fmt::Write for WriteWgr<'_> {
        fn write_str(&mut self, s: &str) -> fmt::Result {
            let mut bytes = test_dbg!(s).as_bytes();
            let rem_skip = test_dbg!(self.skip).saturating_sub(test_dbg!(self.written));
            if test_dbg!(rem_skip) > 0 {
                let len = test_dbg!(bytes.len());
                bytes = if len > rem_skip {
                    self.written += rem_skip;
                    &bytes[rem_skip..]
                } else {
                    self.written += len;
                    &[]
                };
            }

            if !test_dbg!(bytes.is_empty()) {
                let rem_write = self.written - self.skip;
                let buf = &mut self.buf[test_dbg!(rem_write)..];
                let buflen = buf.len();
                if buflen == 0 {
                    // bail
                    return Err(fmt::Error);
                }
                let wrlen = core::cmp::min(buflen, bytes.len());
                buf[..wrlen].copy_from_slice(&bytes[..wrlen]);
                self.written += wrlen;
            }

            Ok(())
        }
    }

    let mut cnt = ByteCtr { cnt: 0 };
    write!(cnt, "{fmt:?}").expect("writing to byte counter should never fail");
    let total_len = cnt.cnt;
    let mut written = 0;

    while written < total_len {
        let rem = total_len - written;
        let mut wgr = producer.send_grant(test_dbg!(rem)).await;
        let written_now = {
            let mut writer = WriteWgr {
                skip: written,
                written: 0,
                buf: &mut wgr[..],
            };
            let _ = write!(&mut writer, "{fmt:?}");
            test_dbg!(writer.written) - test_dbg!(writer.skip)
        };
        written += test_dbg!(written_now);

        wgr.commit(written_now);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const STR: &str = "\
        There was a man named Mord whose surname was Fiddle; he was the \
        son of Sigvat the Red, and he dwelt at the \"Vale\" in the \
        Rangrivervales. He was a mighty chief, and a great taker up of \
        suits, and so great a lawyer that no judgments were thought \
        lawful unless he had a hand in them. He had an only daughter, \
        named Unna. She was a fair, courteous and gifted woman, \
        and that was thought the best match in all the Rangrivervales.\
        ";
    #[tokio::test]
    async fn it_works() {
        let (tx, rx) = crate::new_spsc_channel(16).await;
        let writer = tokio::spawn(async move {
            fmt_to_bbq(&tx, &STR).await;
        });

        let reader = tokio::spawn(async move {
            let len = STR.as_bytes().len();
            let recvd = recv(rx, len).await;
            assert_eq!(recvd, format!("{STR:?}"));
        });

        tokio::time::timeout(tokio::time::Duration::from_secs(60), async move {
            tokio::try_join!(reader, writer).expect("neither task should panic")
        })
        .await
        .expect("reader and writer should complete within 60 seconds");
    }

    #[tokio::test]
    async fn debug_also_works() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct MyCoolStruct<'a> {
            a: &'a str,
            my_vec: Vec<&'a str>,
            c: &'a str,
        }

        let my_struct = MyCoolStruct {
            a: "hello world",
            my_vec: STR.split('.').collect(),
            c: "goodbye world",
        };

        let output = format!("{my_struct:?}");

        let (tx, rx) = crate::new_spsc_channel(16).await;
        let writer = tokio::spawn(async move {
            fmt_to_bbq(&tx, my_struct).await;
        });

        let reader = tokio::spawn(async move {
            let len = output.as_bytes().len();
            let recvd = recv(rx, len).await;
            assert_eq!(recvd, output);
        });

        tokio::time::timeout(tokio::time::Duration::from_secs(60), async move {
            tokio::try_join!(reader, writer).expect("neither task should panic")
        })
        .await
        .expect("reader and writer should complete within 60 seconds");
    }

    async fn recv(rx: crate::Consumer, len: usize) -> String {
        let mut buf = String::with_capacity(len);
        while buf.len() < len {
            let rgr = rx.read_grant().await;
            let len = rgr.len();
            let recv = std::str::from_utf8(&rgr[..len]).expect("must be UTF8");
            println!("recv: {recv:?}");
            buf.push_str(recv);
            rgr.release(len);
        }
        buf
    }
}
