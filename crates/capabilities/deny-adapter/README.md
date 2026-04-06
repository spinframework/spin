This directory contains the deny adapter used to enforce configuration isolation in dependencies when using the component dependencies feature of Spin.

To add a new interface to be adapted:
1. Add the interface to the list in `build.rs`
2. Implement the interface on `Adapter` in `lib.rs`

To build, in the parent crate, run:
```
make adapter
```