use helper::http_trigger_bindings::wasi::http0_2_0::types::{Fields, OutgoingRequest, Scheme};

helper::define_component!(Component);

impl Component {
    fn main() -> Result<(), String> {
        // Set up a request the way Python or Go do it if the URL is authority-form
        let headers = Fields::new();
        let request = OutgoingRequest::new(headers);
        request.set_authority(Some("example.com")).expect("should have set authority");
        request.set_scheme(Some(&Scheme::Https)).expect("should have set scheme");
        request.set_path_with_query(Some("")).expect("should have set PQ to empty");

        Ok(())
    }
}
