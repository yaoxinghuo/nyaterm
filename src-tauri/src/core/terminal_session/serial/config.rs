pub struct SerialConfig {
    pub port_name: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: String,
    pub stop_bits: String,
    pub flow_control: crate::config::SerialFlowControl,
    pub name: String,
    pub backspace_mode: String,
    pub modem_upload_protocol: crate::config::SerialModemUploadProtocol,
    pub encoding: String,
}

fn validate_serial_modem_flow_control(
    flow_control: crate::config::SerialFlowControl,
) -> Result<(), String> {
    if flow_control == crate::config::SerialFlowControl::Software {
        return Err("XMODEM/YMODEM/ZMODEM transfers are disabled with software flow control (XON/XOFF). Use None or Hardware (RTS/CTS) flow control and reconnect.".to_string());
    }
    Ok(())
}
