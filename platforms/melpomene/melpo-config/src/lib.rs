use mnemos_kernel::forth::Params;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct PlatformConfig {
    pub tcp_uart: Option<TcpUartConfig>,
    pub display: Option<DisplayConfig>,
    pub forth_shell: Option<ForthShell>,
    // pub guish: Gui
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TcpUartConfig {
    pub incoming_size: usize,
    pub outgoing_size: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DisplayConfig {
    pub kchannel_depth: usize,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ForthShell {
    pub capacity: usize,
    pub params: Params,
}
