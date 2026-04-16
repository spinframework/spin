This directory contains the deny adapter used to enforce configuration isolation in dependencies when using the component dependencies feature of Spin.

To build, in the parent crate, run:
```
make adapter
```

The target world used to generate these bindings is:

```
world {
  export wasi:http/outgoing-handler@0.2.6;
  export wasi:http/client@0.3.0-rc-2026-03-15;
  export spin:key-value/key-value@3.0.0;
  export spin:mqtt/mqtt@3.0.0;
  export spin:postgres/postgres@3.0.0;
  export spin:postgres/postgres@4.2.0;
  export spin:redis/redis@3.0.0;
  export spin:sqlite/sqlite@3.1.0;
  export spin:variables/variables@3.0.0;
  export wasi:config/store@0.2.0-draft-2024-09-27;
  export fermyon:spin/config;
  export fermyon:spin/http;
  export fermyon:spin/key-value;
  export fermyon:spin/llm;
  export fermyon:spin/mysql;
  export fermyon:spin/postgres;
  export fermyon:spin/redis;
  export fermyon:spin/sqlite;
  export wasi:cli/environment@0.2.6;
  export wasi:filesystem/preopens@0.2.6;
  export wasi:sockets/udp@0.2.6;
  export wasi:sockets/udp-create-socket@0.2.6;
  export wasi:sockets/tcp@0.2.6;
  export wasi:sockets/tcp-create-socket@0.2.6;
  export wasi:sockets/ip-name-lookup@0.2.6;
  export wasi:cli/environment@0.3.0-rc-2026-03-15;
  export wasi:filesystem/preopens@0.3.0-rc-2026-03-15;
  export wasi:sockets/ip-name-lookup@0.3.0-rc-2026-03-15;
  export fermyon:spin/llm@2.0.0;
  export fermyon:spin/redis@2.0.0;
  export fermyon:spin/mqtt@2.0.0;
  export fermyon:spin/postgres@2.0.0;
  export fermyon:spin/mysql@2.0.0;
  export fermyon:spin/sqlite@2.0.0;
  export fermyon:spin/key-value@2.0.0;
  export fermyon:spin/variables@2.0.0;
  export wasi:keyvalue/store@0.2.0-draft2;
}
```