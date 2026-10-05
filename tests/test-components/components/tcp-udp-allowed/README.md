# TCP Sockets

Tests if TCP and UDP are allowed/disallowed at the WASI level.

## Expectations

This test component expects the following to be true:
* It is provided the env variable `EXPECTED_TO_ALLOW` with value `true` or `false`
* If the env variable is `true` then opening a socket should succeed; if the variable is `false` then opening a socket should fail
