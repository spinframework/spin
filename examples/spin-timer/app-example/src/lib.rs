wit_bindgen::generate!({
    world: "spin-timer",
    path: "..",
    generate_all,
});

use fermyon::spin::variables;

struct MySpinTimer;
export!(MySpinTimer);

impl Guest for MySpinTimer {
    fn handle_timer_request() {
        let text = variables::get("message").unwrap();
        println!("{text}");
    }
}
