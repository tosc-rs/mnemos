use crate::comms::bbq::BBQBidiHandle;
use spitebuf::MpMcQueue;
pub struct SerialMux {
    serial_port: BBQBidiHandle,
}
