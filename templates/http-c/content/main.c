#include "bindings/http_trigger.h"

void exports_wasi_http_0_2_0_incoming_handler_handle(
    exports_wasi_http_0_2_0_incoming_handler_own_incoming_request_t request,
    exports_wasi_http_0_2_0_incoming_handler_own_response_outparam_t response_out
) {
    wasi_http_0_2_0_types_borrow_incoming_request_t req_b = wasi_http_0_2_0_types_borrow_incoming_request(request);

    wasi_http_0_2_0_types_own_headers_t headers = wasi_http_0_2_0_types_constructor_fields();
    wasi_http_0_2_0_types_own_outgoing_response_t resp = wasi_http_0_2_0_types_constructor_outgoing_response(headers);
    wasi_http_0_2_0_types_borrow_outgoing_response_t resp_b = wasi_http_0_2_0_types_borrow_outgoing_response(resp);

    wasi_http_0_2_0_types_own_outgoing_body_t ogbod;
    if (!wasi_http_0_2_0_types_method_outgoing_response_body(resp_b, &ogbod)) {
        return;
    }
    wasi_http_0_2_0_types_borrow_outgoing_body_t ogbod_b = wasi_http_0_2_0_types_borrow_outgoing_body(ogbod);

    wasi_http_0_2_0_types_result_own_outgoing_response_error_code_t respe = {
        .is_err = false,
        .val = resp,
    };
    wasi_http_0_2_0_types_static_response_outparam_set(response_out, &respe);

    http_trigger_string_t pq;
    if (wasi_http_0_2_0_types_method_incoming_request_path_with_query(req_b, &pq)) {
        wasi_http_0_2_0_types_own_output_stream_t out_stm;
        if (wasi_http_0_2_0_types_method_outgoing_body_write(ogbod_b, &out_stm)) {
            wasi_io_0_2_0_streams_borrow_output_stream_t out_stm_b = wasi_io_0_2_0_streams_borrow_output_stream(out_stm);
            wasi_io_0_2_0_streams_stream_error_t err;

            http_trigger_list_u8_t contents = { .ptr = pq.ptr, .len = pq.len };
            wasi_io_0_2_0_streams_method_output_stream_blocking_write_and_flush(out_stm_b, &contents, &err);

            char* nl = "\n";
            http_trigger_list_u8_t nl_contents = { .ptr = (uint8_t*)nl, .len = 1 };
            wasi_io_0_2_0_streams_method_output_stream_blocking_write_and_flush(out_stm_b, &nl_contents, &err);

            wasi_io_0_2_0_streams_output_stream_drop_own(out_stm);
        }
    }

    wasi_http_0_2_0_types_error_code_t err_code;
    wasi_http_0_2_0_types_static_outgoing_body_finish(ogbod, NULL, &err_code);
}
