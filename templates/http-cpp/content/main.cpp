#include "bindings/wit.h"
#include "bindings/http_trigger_cpp.h"
#include "bindings/wit.h"
#include <span>

using wasi::http0_2_0::types::Fields;
using wasi::http0_2_0::types::IncomingRequest;
using wasi::http0_2_0::types::OutgoingBody;
using wasi::http0_2_0::types::OutgoingResponse;
using wasi::http0_2_0::types::ResponseOutparam;

namespace conv {
    std::span<const uint8_t> spanify(const char* chs, size_t sz) {
        return std::span(reinterpret_cast<uint8_t*>(const_cast<char*>(chs)), sz);
    }
    std::span<const uint8_t> spanify(const char* chs) {
        return spanify(chs, strlen(chs));
    }
    std::span<const uint8_t> spanify(wit::string s) {
        const auto len = s.size();
        const auto chars = s.data();
        return spanify(chars, len);
    }
}

namespace exports::wasi::http0_2_0::incoming_handler {
    void Handle(IncomingRequest&& request, ResponseOutparam&& response_out) {
        Fields headers;
        OutgoingResponse resp(std::move(headers));
        auto ogbod = resp.Body().value();
        ResponseOutparam::Set(std::move(response_out), std::move(resp));

        auto pq = request.PathWithQuery();

        if (pq.has_value()) {
            const auto pq_text = pq.value();

            auto out_stm = ogbod.Write().value();
            out_stm.BlockingWriteAndFlush(conv::spanify(pq_text));
            out_stm.BlockingWriteAndFlush(conv::spanify("\n"));
        } else {
            auto out_stm = ogbod.Write().value();
            out_stm.BlockingWriteAndFlush(conv::spanify("no path-and-query\n"));
        }

        OutgoingBody::Finish(std::move(ogbod), std::nullopt);
    }
}
