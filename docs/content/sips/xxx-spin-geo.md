title = "SIP xxx - Spin Geo interface"
template = "main"
date = "2025-08-19T12:00:00Z"

---

Summary: Introduce new `spin:geo` interface

Owner: lann.martin@fermyon.com

Created: Aug 19, 2025

## Background

TODO

## Proposal

### WIT Interfaces

```wit
package spin:geo;

interface types {
  struct position {
    // WGS 84 (i.e. GPS) coordinates
    latitude: f32;
    longitude: f32;
    // Estimated radius around coordinates of actual location, in meters
    accuracy: option<f32>;
  }

  // Represents a geographic location.
  resource location {
    // A geolocation position
    position: func() -> option<position>;

    // ISO 3166-1 alpha-2 code (uppercase)
    country-code: func() -> option<string>;

    // The three letter part of a UN/LOCODE code (uppercase)
    // Note: This will often match a nearby IATA airport code.
    local-code: func() -> option<string>;
  }
}

interface host {
  struct provider-region {
    // The name of the provider; see:
    // https://opentelemetry.io/docs/specs/semconv/registry/attributes/cloud/#cloud-provider
    provider: string,

    // A provider-specific region identifier; see:
    // https://opentelemetry.io/docs/specs/semconv/registry/attributes/cloud/#cloud-region
    region: string
  }

  // Returns the location of the host running this process.
  location: func() -> option<location>;

  provider-region: func() -> option<provider-region>;
}

interface internet {
  variant error {
    // No location information found for the given address
    not-found,
    // The address was malformed or not a public internet address
    invalid-ip-address,
    // Some implementation-specific error occurred
    other(string),
  }

  // Returns a location for the given IP address.
  lookup: func(ip-address: string) -> result<location, error>;
}
```