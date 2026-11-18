//! Thin Bluetooth Classic (RFCOMM / SPP) layer.
//!
//! Earbuds and headphones expose their settings over an RFCOMM serial stream
//! rather than HID (EarFun = Qualcomm GAIA, and so on). This module only lists
//! paired devices and moves bytes; the meaning of each frame belongs to the
//! drivers (`earfun.rs`, ...).
//!
//! Windows only (Winsock `AF_BTH`). On other systems the list is empty and
//! connecting reports `Unsupported`, so the crate still builds.

use devhub_core::backend::{HubError, HubResult};
use std::time::Duration;

/// One paired Bluetooth device.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BtDevice {
    /// 48-bit address (upper 16 bits are zero).
    pub addr: u64,
    pub name: String,
    /// Whether a link is up right now (powered on and connected).
    pub connected: bool,
    /// Class of Device.
    pub class: u32,
}

/// `AA:BB:CC:DD:EE:FF` notation.
pub fn addr_string(addr: u64) -> String {
    let b = addr.to_be_bytes();
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        b[2], b[3], b[4], b[5], b[6], b[7]
    )
}

/// A 16-bit short UUID placed on the Bluetooth base UUID.
pub const fn uuid16(short: u16) -> u128 {
    0x0000_0000_0000_1000_8000_0080_5F9B_34FB | ((short as u128) << 96)
}

/// Serial Port Profile.
pub const UUID_SPP: u128 = uuid16(0x1101);

#[cfg(windows)]
mod imp {
    use super::*;
    use std::sync::Once;
    use windows_sys::core::GUID;
    use windows_sys::Win32::Devices::Bluetooth::{
        BluetoothFindDeviceClose, BluetoothFindFirstDevice, BluetoothFindNextDevice, AF_BTH,
        BLUETOOTH_DEVICE_INFO, BLUETOOTH_DEVICE_SEARCH_PARAMS, BTHPROTO_RFCOMM, SOCKADDR_BTH,
    };
    use windows_sys::Win32::Networking::WinSock::{
        closesocket, connect, ioctlsocket, recv, select, send, socket, WSAGetLastError, WSAStartup,
        FD_SET, FIONBIO, INVALID_SOCKET, SOCKADDR, SOCKET, SOCKET_ERROR, SOCK_STREAM, TIMEVAL,
        WSADATA, WSAEWOULDBLOCK,
    };

    fn startup() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| unsafe {
            let mut data: WSADATA = std::mem::zeroed();
            WSAStartup(0x0202, &mut data);
        });
    }

    fn guid(u: u128) -> GUID {
        let b = u.to_be_bytes();
        GUID {
            data1: u32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            data2: u16::from_be_bytes([b[4], b[5]]),
            data3: u16::from_be_bytes([b[6], b[7]]),
            data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
        }
    }

    pub fn paired() -> HubResult<Vec<BtDevice>> {
        let params = BLUETOOTH_DEVICE_SEARCH_PARAMS {
            dwSize: std::mem::size_of::<BLUETOOTH_DEVICE_SEARCH_PARAMS>() as u32,
            fReturnAuthenticated: 1,
            fReturnRemembered: 1,
            fReturnUnknown: 0,
            fReturnConnected: 1,
            // No inquiry: only the list the OS already remembers, so this is fast.
            fIssueInquiry: 0,
            cTimeoutMultiplier: 0,
            hRadio: std::ptr::null_mut(),
        };
        let mut info: BLUETOOTH_DEVICE_INFO = unsafe { std::mem::zeroed() };
        info.dwSize = std::mem::size_of::<BLUETOOTH_DEVICE_INFO>() as u32;
        let mut out = Vec::new();
        // Null is returned both without a radio and with zero devices; either
        // way there is nothing to list.
        let find = unsafe { BluetoothFindFirstDevice(&params, &mut info) };
        if find.is_null() {
            return Ok(out);
        }
        loop {
            let len = info.szName.iter().position(|c| *c == 0).unwrap_or(248);
            out.push(BtDevice {
                addr: unsafe { info.Address.Anonymous.ullLong } & 0xFFFF_FFFF_FFFF,
                name: String::from_utf16_lossy(&info.szName[..len]),
                connected: info.fConnected != 0,
                class: info.ulClassofDevice,
            });
            if unsafe { BluetoothFindNextDevice(find, &mut info) } == 0 {
                break;
            }
        }
        unsafe { BluetoothFindDeviceClose(find) };
        Ok(out)
    }

    pub struct Rfcomm {
        sock: SOCKET,
    }

    // A SOCKET is a plain handle. The driver's mutex serialises send/recv.
    unsafe impl Send for Rfcomm {}
    unsafe impl Sync for Rfcomm {}

    fn wait(sock: SOCKET, write: bool, timeout: Duration) -> HubResult<bool> {
        let mut set: FD_SET = unsafe { std::mem::zeroed() };
        set.fd_count = 1;
        set.fd_array[0] = sock;
        let mut err = set;
        let tv = TIMEVAL {
            tv_sec: timeout.as_secs() as i32,
            tv_usec: timeout.subsec_micros() as i32,
        };
        let null = std::ptr::null_mut::<FD_SET>();
        let n = unsafe {
            if write {
                select(0, null, &mut set, &mut err, &tv)
            } else {
                select(0, &mut set, null, &mut err, &tv)
            }
        };
        if n == SOCKET_ERROR {
            return Err(HubError::Io(format!("bt select: {}", unsafe {
                WSAGetLastError()
            })));
        }
        if n > 0 && err.fd_count > 0 && set.fd_count == 0 {
            return Err(HubError::Disconnected);
        }
        Ok(n > 0)
    }

    impl Rfcomm {
        fn open(addr: u64, service: GUID, port: u32, timeout: Duration) -> HubResult<Self> {
            startup();
            let sock = unsafe { socket(AF_BTH as i32, SOCK_STREAM, BTHPROTO_RFCOMM as i32) };
            if sock == INVALID_SOCKET {
                return Err(HubError::Io(format!("bt socket: {}", unsafe {
                    WSAGetLastError()
                })));
            }
            let this = Self { sock };
            // A blocking connect to an out-of-range peer does not return for
            // tens of seconds, so start it non-blocking and wait with select.
            let mut on = 1u32;
            unsafe { ioctlsocket(sock, FIONBIO, &mut on) };
            let sa = SOCKADDR_BTH {
                addressFamily: AF_BTH,
                btAddr: addr,
                serviceClassId: service,
                port,
            };
            let rc = unsafe {
                connect(
                    sock,
                    &sa as *const SOCKADDR_BTH as *const SOCKADDR,
                    std::mem::size_of::<SOCKADDR_BTH>() as i32,
                )
            };
            if rc == SOCKET_ERROR {
                let e = unsafe { WSAGetLastError() };
                if e != WSAEWOULDBLOCK {
                    return Err(HubError::Io(format!("bt connect: {e}")));
                }
                match wait(sock, true, timeout) {
                    Ok(true) => {}
                    Ok(false) => return Err(HubError::Io("bt connect: timeout".into())),
                    Err(HubError::Disconnected) => {
                        return Err(HubError::Io("bt connect: refused".into()))
                    }
                    Err(e) => return Err(e),
                }
            }
            let mut off = 0u32;
            unsafe { ioctlsocket(sock, FIONBIO, &mut off) };
            Ok(this)
        }

        /// Connect by service class UUID (the OS resolves the channel via SDP).
        pub fn connect(addr: u64, service: u128, timeout: Duration) -> HubResult<Self> {
            Self::open(addr, guid(service), 0, timeout)
        }

        /// Connect to an explicit RFCOMM channel (for channels missing from SDP).
        pub fn connect_channel(addr: u64, channel: u8, timeout: Duration) -> HubResult<Self> {
            Self::open(addr, guid(0), channel as u32, timeout)
        }

        pub fn send(&self, data: &[u8]) -> HubResult<()> {
            let mut off = 0usize;
            while off < data.len() {
                let n = unsafe {
                    send(
                        self.sock,
                        data[off..].as_ptr(),
                        (data.len() - off) as i32,
                        0,
                    )
                };
                if n == SOCKET_ERROR || n == 0 {
                    return Err(HubError::Disconnected);
                }
                off += n as usize;
            }
            Ok(())
        }

        /// Read whatever arrives within `timeout`. 0 = timed out with no data.
        pub fn recv(&self, buf: &mut [u8], timeout: Duration) -> HubResult<usize> {
            if !wait(self.sock, false, timeout)? {
                return Ok(0);
            }
            let n = unsafe { recv(self.sock, buf.as_mut_ptr(), buf.len() as i32, 0) };
            // 0 = the peer closed the link (power off or reboot).
            if n == SOCKET_ERROR || n == 0 {
                return Err(HubError::Disconnected);
            }
            Ok(n as usize)
        }
    }

    impl Drop for Rfcomm {
        fn drop(&mut self) {
            unsafe { closesocket(self.sock) };
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn paired() -> HubResult<Vec<BtDevice>> {
        Ok(Vec::new())
    }

    pub struct Rfcomm;

    impl Rfcomm {
        pub fn connect(_addr: u64, _service: u128, _timeout: Duration) -> HubResult<Self> {
            Err(HubError::Unsupported("Bluetooth on this OS"))
        }
        pub fn connect_channel(_addr: u64, _channel: u8, _timeout: Duration) -> HubResult<Self> {
            Err(HubError::Unsupported("Bluetooth on this OS"))
        }
        pub fn send(&self, _data: &[u8]) -> HubResult<()> {
            Err(HubError::Disconnected)
        }
        pub fn recv(&self, _buf: &mut [u8], _timeout: Duration) -> HubResult<usize> {
            Err(HubError::Disconnected)
        }
    }
}

pub use imp::{paired, Rfcomm};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uuid_and_addr_format() {
        assert_eq!(UUID_SPP, 0x0000_1101_0000_1000_8000_0080_5F9B_34FB);
        assert_eq!(addr_string(0x0011_2233_AABB), "00:11:22:33:AA:BB");
    }
}
