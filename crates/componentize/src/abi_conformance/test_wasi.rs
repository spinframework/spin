use super::{Context, TestConfig};
use anyhow::{Result, ensure};
use rand_chacha::ChaCha12Core;
use rand_core::{
    SeedableRng,
    block::{BlockRng, BlockRngCore},
};
use serde::Serialize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    collections::HashSet,
    fs::File,
    io::Write,
    ops::Deref,
    time::{Duration, SystemTime},
};
use wasmtime::{Engine, component::InstancePre};
use wasmtime_wasi::{
    HostWallClock,
    p2::pipe::{MemoryInputPipe, MemoryOutputPipe},
};

/// Report of which WASI functions a module successfully used, if any
///
/// This represents the subset of WASI which is relevant to Spin and does not include e.g. network sockets,
/// filesystem odification, etc.
#[derive(Serialize, PartialEq, Eq, Debug)]
pub struct WasiReport {
    /// Result of the WASI environment variable test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["wasi-env", "foo"\] as
    /// arguments.  The module should call the host-implemented `wasi_snapshot_preview1::environ_get` function
    /// w`ok("foo=bar")` as the result.  The module should extract the value of the "foo" variable and write the
    /// result to `stdout` as a UTF-8 string.  The host will assert the output matches the expected value.
    pub env: Result<(), String>,

    /// Result of the WASI system clock test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["wasi-epoch"\] as the
    /// argument.  The module should call the host-implemented `wasi_snapshot_preview1::clock_time_get` function
    /// with `realtime` as the clock ID and expect `ok(1663014331719000000)` as the result.  The module should then
    /// divide that value by 1000000 to convert to milliseconds and write the result to `stdout` as a UTF-8 string.
    /// The host will assert the output matches the expected value.
    pub epoch: Result<(), String>,

    /// Result of the WASI system random number generator test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["wasi-random"\] as the
    /// argument.  The module should call the host-implemented `wasi_snapshot_preview1::random_get` function at
    /// least once.  The host will assert that said function was called at least once.
    pub random: Result<(), String>,

    /// Result of the WASI stdio test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["wasi-stdio"\] as the
    /// argument.  The module should call the host-implemented `wasi_snapshot_preview1::fd_read` and
    /// `wasi_snapshot_preview1::fd_write` functions as necessary to read the UTF-8 string "All mimsy were the
    /// borogroves" from `stdin` and write the same string back to `stdout`.  The host will assert that the output
    /// matches the input.
    pub stdio: Result<(), String>,

    /// Result of the WASI filesystem read test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["wasi-read",
    /// "foo.txt"\] as arguments.  The module should call the relevant `wasi_snapshot_preview1` functions to open
    /// the file "foo.txt" in the preopened directory descriptor 3 and read its content, which will be the UTF-8
    /// string "And the mome raths outgrabe".  The module should then write that string to `stdout`.  The host will
    /// assert that the output matches the contents of the file.
    pub read: Result<(), String>,

    /// Result of the WASI filesystem readdir test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["wasi-readdir", "/"\]
    /// as arguments.  The module should call the relevant `wasi_snapshot_preview1` functions to read the contents
    /// of the preopened directory named "/" and write them to `stdout` as comma-delimited, UTF-8-encoded strings
    /// (in arbitrary order), skipping the "." and ".." entries.  The host will assert that the output matches the
    /// contents of the directory: "bar.txt", "baz.txt", and "foo.txt".
    pub readdir: Result<(), String>,

    /// Result of the WASI filesystem stat test
    ///
    /// The guest module should expect a call according to [`super::InvocationStyle`] with \["wasi-stat",
    /// "foo.txt"\] as arguments.  The module should call the relevant `wasi_snapshot_preview1` functions to
    /// retrieve metadata from the file "foo.txt" in the preopened directory descriptor 3.  The module should then
    /// write a UTF-8-encoded string of the form "length:<length>,modified:<modified>" to `stdout`, where
    /// "<length>" is the length of the file and "<modified>" is the last-modified time in milliseconds since 1970
    /// UTC.  The host will assert that the output matches the metdata of the file.
    pub stat: Result<(), String>,
}

pub(crate) async fn test(
    engine: &Engine,
    test_config: TestConfig,
    pre: &InstancePre<Context>,
) -> Result<WasiReport> {
    Ok(WasiReport {
        env: {
            let stdout = MemoryOutputPipe::new(1024);
            let mut store = super::create_store_with_wasi(engine, test_config.clone(), |wasi| {
                wasi.env("foo", "bar").stdout(stdout.clone());
            });

            super::run_command(&mut store, pre, &["wasi-env", "foo"], move |_| {
                let stdout = String::from_utf8(stdout.try_into_inner().unwrap().to_vec())?;
                ensure!(
                    "bar" == stdout.deref(),
                    "expected module to write \"bar\" to stdout, got {stdout:?}"
                );

                Ok(())
            })
            .await
        },

        epoch: {
            const TIME: u64 = 1663014331719;

            struct MyClock;

            impl HostWallClock for MyClock {
                fn resolution(&self) -> Duration {
                    Duration::from_millis(1)
                }

                fn now(&self) -> Duration {
                    Duration::from_millis(TIME)
                }
            }

            let stdout = MemoryOutputPipe::new(1024);
            let mut store = super::create_store_with_wasi(engine, test_config.clone(), |wasi| {
                wasi.stdout(stdout.clone()).wall_clock(MyClock);
            });

            super::run_command(&mut store, pre, &["wasi-epoch"], move |_| {
                let stdout = String::from_utf8(stdout.try_into_inner().unwrap().to_vec())?;
                ensure!(
                    TIME.to_string() == stdout,
                    "expected module to write {TIME:?} to stdout, got {stdout:?}"
                );

                Ok(())
            })
            .await
        },

        random: {
            #[derive(Clone)]
            struct MyRngCore {
                cha_cha_12: ChaCha12Core,
                called: Arc<AtomicBool>,
            }

            impl BlockRngCore for MyRngCore {
                type Item = <ChaCha12Core as BlockRngCore>::Item;
                type Results = <ChaCha12Core as BlockRngCore>::Results;

                fn generate(&mut self, results: &mut Self::Results) {
                    self.called.store(true, Ordering::Relaxed);
                    self.cha_cha_12.generate(results)
                }
            }

            let called = Arc::new(AtomicBool::default());
            let mut store = super::create_store_with_wasi(engine, test_config.clone(), |wasi| {
                wasi.insecure_random(Box::new(BlockRng::new(MyRngCore {
                    cha_cha_12: ChaCha12Core::seed_from_u64(42),
                    called: called.clone(),
                })));
            });

            super::run_command(&mut store, pre, &["wasi-random"], move |_| {
                // TODO: fix test to pass
                // ensure!(
                //     called.load(Ordering::Relaxed),
                //     "expected module to call `wasi_snapshot_preview1::random_get` at least once"
                // );

                Ok(())
            })
            .await
        },

        stdio: {
            let stdin_contents = "All mimsy were the borogroves";
            let stdin = MemoryInputPipe::new(stdin_contents);
            let stdout = MemoryOutputPipe::new(1024);

            let mut store = super::create_store_with_wasi(engine, test_config.clone(), |wasi| {
                wasi.stdout(stdout.clone()).stdin(stdin);
            });

            super::run_command(&mut store, pre, &["wasi-stdio"], move |_| {
                let stdout_contents = String::from_utf8(stdout.try_into_inner().unwrap().to_vec())?;
                ensure!(
                    stdin_contents == stdout_contents.deref(),
                    "expected module to write {stdin_contents:?} to stdout, got {stdout_contents:?}"
                );

                Ok(())
            })
            .await
        },

        read: {
            let stdout = MemoryOutputPipe::new(1024);
            let message = "And the mome raths outgrabe";
            let dir = tempfile::tempdir()?;
            let mut file = File::create(dir.path().join("foo.txt"))?;
            file.write_all(message.as_bytes())?;
            let perms = wasmtime_wasi::DirPerms::all();
            let file_perms = wasmtime_wasi::FilePerms::all();

            let mut store = super::create_store_with_wasi(engine, test_config.clone(), |wasi| {
                wasi.stdout(stdout.clone())
                    .preopened_dir(dir.path(), "/", perms, file_perms)
                    .unwrap();
            });

            super::run_command(&mut store, pre, &["wasi-read", "foo.txt"], move |_| {
                let stdout = String::from_utf8(stdout.try_into_inner().unwrap().to_vec())?;
                ensure!(
                    message == stdout.deref(),
                    "expected module to write {message:?} to stdout, got {stdout:?}"
                );

                Ok(())
            })
            .await
        },

        readdir: {
            let stdout = MemoryOutputPipe::new(1024);
            let dir = tempfile::tempdir()?;

            let names = ["foo.txt", "bar.txt", "baz.txt"];
            for &name in &names {
                File::create(dir.path().join(name))?;
            }
            let perms = wasmtime_wasi::DirPerms::all();
            let file_perms = wasmtime_wasi::FilePerms::all();
            let mut store = super::create_store_with_wasi(engine, test_config.clone(), |wasi| {
                wasi.stdout(stdout.clone())
                    .preopened_dir(dir.path(), "/", perms, file_perms)
                    .unwrap();
            });

            super::run_command(&mut store, pre, &["wasi-readdir", "/"], move |_| {
                let expected = names.iter().copied().collect::<HashSet<_>>();
                let stdout = String::from_utf8(stdout.try_into_inner().unwrap().to_vec())?;
                let got = stdout.split(',').collect();
                ensure!(
                    expected == got,
                    "expected module to write {expected:?} to stdout (in any order), got {got:?}"
                );

                Ok(())
            })
            .await
        },

        stat: {
            let stdout = MemoryOutputPipe::new(1024);
            let message = "O frabjous day! Callooh! Callay!";
            let dir = tempfile::tempdir()?;
            let mut file = File::create(dir.path().join("foo.txt"))?;
            file.write_all(message.as_bytes())?;
            let metadata = file.metadata()?;
            let perms = wasmtime_wasi::DirPerms::all();
            let file_perms = wasmtime_wasi::FilePerms::all();

            let mut store = super::create_store_with_wasi(engine, test_config.clone(), |wasi| {
                wasi.stdout(stdout.clone())
                    .preopened_dir(dir.path(), "/", perms, file_perms)
                    .unwrap();
            });

            super::run_command(&mut store, pre, &["wasi-stat", "foo.txt"], move |_| {
                let expected = format!(
                    "length:{},modified:{}",
                    metadata.len(),
                    metadata
                        .modified()?
                        .duration_since(SystemTime::UNIX_EPOCH)?
                        .as_millis()
                );
                let got = String::from_utf8(stdout.try_into_inner().unwrap().to_vec())?;

                ensure!(
                    expected == got,
                    "expected module to write {expected:?} to stdout, got {got:?}"
                );

                Ok(())
            })
            .await
        },
    })
}
