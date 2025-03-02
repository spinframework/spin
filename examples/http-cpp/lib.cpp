#include <cstdlib>
#include <cstring>

#include "spin-http.h"

extern "C" void spin_http_handle_http_request(spin_http_request_t *request,
                                              spin_http_response_t *response) {
  spin_http_string_t header_name;
  spin_http_string_dup(&header_name, "foo");

  spin_http_string_t header_value;
  spin_http_string_dup(&header_value, "bar");

  auto header = static_cast<spin_http_tuple2_string_string_t *>(
      malloc(sizeof(spin_http_tuple2_string_string_t)));

  header->f0 = header_name;
  header->f1 = header_value;

  auto body_string = "Hello World!\n";
  auto body_length = strlen(body_string);
  auto body = static_cast<uint8_t *>(malloc(body_length));
  memcpy(body, body_string, body_length);

  response->status = 200;
  response->headers.is_some = true;
  response->headers.val.ptr = header;
  response->headers.val.len = 1;
  response->body.is_some = true;
  response->body.val.ptr = body;
  response->body.val.len = body_length;
}
