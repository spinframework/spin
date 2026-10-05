use helper::http_trigger_bindings::wasi::{
    sockets0_2_0::{
        network::{
            IpAddressFamily,
        },
        tcp_create_socket,
        udp_create_socket,
    },
};
use helper::{ensure, ensure_ok};

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        let expected_to_allow: bool = ensure_ok!(ensure_ok!(std::env::var("EXPECTED_TO_ALLOW")).parse());

        let tcp_result = tcp_create_socket::create_tcp_socket(IpAddressFamily::Ipv4);
        if expected_to_allow {
            ensure_ok!(tcp_result);
        } else {
            ensure!(tcp_result.is_err());
        }

        let udp_result = udp_create_socket::create_udp_socket(IpAddressFamily::Ipv4);
        if expected_to_allow {
            ensure_ok!(udp_result);
        } else {
            ensure!(udp_result.is_err());
        }

        Ok(())
    }
}
