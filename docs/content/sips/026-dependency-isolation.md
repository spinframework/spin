title = "SIP 026 - Dependency isolation"
template = "main"
date = "2026-09-02T00:00:00Z"
---

Summary: Enable dependency capabilities that are independent of the parent component.

## Context

Spin component dependencies want to be able to access resources such as configuration variables, databases, and remote APIs.
However, until recently, the component model has enforced that if a composition imports an interface, then those
imports are indistiguishable: all imports of, say, the `spin:sqlite/sqlite` interface get the exact same
host implementation.

The component model has now added an "implements" feature, which allows a component (for our purposes, a composition)
to import an interface under a different name:

```
world w {
    import my-db: spin:sqlite/sqlite;
}
```

The host must mark the interface as `named_imports` in `wasmtime::component::bindgen!`: this generates additional,
parallel `Host` traits which are the same as the normal host traits with an extra ID parameter (roughly, the name
under which the guest imported the interface).

## How does this help?

Consider a situation where a primary component wants to access the `finance` database, and a dependency
wants to access the `customer` database:

```toml
# hypothetical
[component.shop]
sqlite_databases = ["finance"]
[component.shop.dependencies]
"customer:customer/account" = { path = "...", capabilities = { sqlite_databases = ["customer"] } }
```

The naive composition graph looks like this. Note how the host sees only one import of the interface.

```
{----------------------- Wasm binary -------------------}

[ main comp  ] <---
                   \
                    --- [ import spin:sqlite/sqlite ]
                   /
[ dependency ] <---
```

The idea is the during composition, we generate a nemed import of `spin:sqlite/sqlite`, and plug
that onto the dependency:

```
{--------------------- Wasm binary -----------------}           {-------- synthesised by Spin --------}

[ main comp  ] <--- [ import spin:sqlite/sqlite ]

[ dependency ] <--- [ import spin:sqlite/sqlite ] <-- PLUG IN -- [ import custo: spin:sqlite/sqlite ]
```

Now the host sees separate named and unnamed imports of the interface.

> Note that the _dependency_ is not aware of the named import. As far as the dependency is concerned,
> it's still importing `spin:sqlite/sqlite`. The named import is a composition-time artefact which
> is outside what the dependency can know.

The host can now distinguish the dependency's import from the primary component's import, and can
apply different validation rules to each.

> In principle, the host could supply completely different implementations to each. That's not useful
> here: Spin only cares about restricting access.

We can do this by extending the behaviour of the existing capabilities code. Currently, the capabilities
code is limited to composing a deny adapter onto a dependency import. The new behaviour would be, for each
interface:

* Is there an isolated capability for the interface? If so, compose a named import onto the interface. Otherwise:
* Is there an inherit specified for the interface? If so, do nothing (as today). Otherwise:
* Compose the deny adapter onto the interface (as today).

## Named imports and capability sets

The actual name of the named import doesn't matter: it is a private contract between the capabilities
composer and the host trait. What matters is:

1. Dependencies should not be able to forge import names. If a malicious developer crafts a component
   which named-imports `super-secret-db-password`, and persuades me to use it in a composition,
   Spin should not mistake the crafted import for an application-granted permission to the
   super secret DB password.
2. Spin must be able to map the import name - which is, roughly, what we get at the host trait - to
   a capability set defined in the manifest. 

So as an initial proposal, I suggest that we name the imports using everybody's favourite highly
readable keys, GUIDs. That said, we may want something more debuggable in the vanishingly unlikely
event that this doesn't work infallibly first time - perhaps a combination of a GUID for unpredictability
with an identifying string for tracing back to what it actually represents. We can iterate on this.

My initial sense is that a key will correspond to a capability and be dependency scoped - for example,
a key for the `sqlite_databases` capability on the `foo` dependency, a key for the the `key_value_stores`
capability on the `foo` dependency, a key for the `sqlite_databases` capability on the `bar` dependency.
This makes point 2, mapping keys to capability sets, easier. But because a capability
(`allowed_outbound_hosts`) can enable multiple interfaces, we will need
to further distinguish named imports by interface.  We therefore end up with:

* capability set key (CSK): identifies a specific capability set (AOH on `foo`, sqlite on `bar`)
* named import key: identifies a specific named import (`foo`'s import of `spin:postgres`)

The host trait will receive the _named import key_.  Therefore, it must be possible to derive
the capability set key from the named import key (as the capability set key is what we use to
look up capability sets).

Because the CSKs are not predictable ones like database names, we will need to maintain a separate
map of CSKs to capability sets. The flow here is something like:

* When loading a component, generate a CSK for each combination of dep and capability,
  and record this in the lockfile.
* When composing a component, pass the CSKs into the composition engine to generate the named imports.
  * This applies whether precomposing for `registry push` or "we're doing it live" composition in the trigger
  * This should mean than everything Just Works TM for running applications from OCI
* Each factor can access the CSK-to-capability set map during `configure_app`, and should record
  the mappings that interest it
  * During instance building, the factor can grab the map for the current component from its saved app state
    and stash them in the instance state 

With this in place, when the factor handles an import via its named import host trait:

* It derives the CSK from the named import key (received via the trait method)
* It looks up the CSK in the instance state, and gets the capability set
* It verifies access against that capability set (instead of against the primary component capabilities)
  * This is the exact same behaviour as normal access validation, just against a different list.
    It returns the same errors etc.

> An alternative approach is to have a per-dependency instance state, which is potentially
> tidier if the named import host simply looks up the sub-instance-state and invokes that
> as if it were a host. However this results in challenges around resource sharing (e.g. does
> a dependency use the same connection pool as its parent). My current feeling is that this
> isn't worth it but I'm happy to be talked around.

Some of this is a little more fiddly with `allowed_outbound_hosts` because we need to talk to another
factor for verification, but it still works out (terms and conditions apply: see below).

## Setting up the named import host traits

To set up the named import host traits, we will add a `named_imports` section to the `spin_world`
`wasmtime::component::bindgen!` macro. This creates a parallel set of host traits to the normal
ones, identical except for an extra "named import ID" parameter.

Linking these is, unfortunately, a bit of a pain, because linking requires the names of the imports,
which of course we don't know statically.  The way I have tried this is adding a new `Factor` trait
method, `register_named_imports`. This must be called for each component: we can do this during
pre-instantiation.

> An alternative approach would be to predict the named import keys from the CSKs in the `LockedApp`,
> and link those directly. This would avoid the need to have the parsed `Component` at link time.
> But it would still need the `LockedApp`, and therefore still need a separate factor method.
> (Or maybe we could add a linker to the `ConfigureAppContext`? It all seems much of a muchness.)

## Wait, you promised me terms and conditions

The one problem with all this is WASI, which is admittedly a pretty ginormogantic problem as
problems for a WASI host go.

Spin doesn't implement the WASI interfaces: instead it hands off to the `wasmtime_wasi` and
`wasmtime_wasi_http` crates. And these do not implement named imports.

If we add, say, `wasi:http/handler` to the `spin_world` `named_imports`, we do get something,
but the something is unwieldy: we have to implement every HTTP host trait, using a set of
types which is separate from the `wasmtime_wasi_http` ones. And we have to link those traits,
which is a bit of a dark art in itself given `WasiCtx` and `WasiCtxView` and Uncle Tom Cobbley
and all.

My proposed solution to this is to go cap in hand to Wasmtime and ask them to do it. We
won't be the only host to hit this problem: it's better addressed in the crates, where
they can share types and include methods in hooks and all the stuff that we'd have to
replicate to make it work.

## Other considerations

### Files

Should a dependency be able to list files?  If so, we need to do a bit more work:

* Mount the files into an asset directory separate from the main component's
* Map filesystem named imports to they access only that directory
* Include the dep files in `spin registry push` and `pull`

### Mixing inheritance and custom capabilities

The current proof of concept applies _either_ inheritance _or_ custom capabilities
during composition. We should consider if we want to allow mixing, e.g. if a
parent wants to isolate the dep's storage and network permissions, but inherit
variables or environment variables.

### Targets / host requirements

Specifying custom capabilities will be a change to the manifest and require runtime
support. We might need to declare a hostreq for it so that devs get warnings when
using it when targeting hosts that don't yet have it. (In principle it will fail
at deploy time because the manifest will fail to parse... but the dev may want a
heads up earlier than that... that was what motivated the decision for middleware.)
