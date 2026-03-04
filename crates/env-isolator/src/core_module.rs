//! Core module generation for environment variable prefix filtering.
//!
//! # Architecture
//!
//! This module generates two core Wasm modules that work together:
//!
//! 1. **Memory module** ([`build_memory_module`]): Owns the linear memory and provides
//!    a bump allocator (`realloc`) plus a `reset` function. The bump allocator is
//!    intentionally simple — it never frees individual allocations. Instead, the
//!    *entire* heap is recycled by calling `reset`, which moves the bump pointer back
//!    to the start of the heap. Memory growth (`memory.grow`) is handled automatically
//!    when an allocation would exceed the current memory size.
//!
//! 2. **Filter module** ([`generate_env_filter_module`]): Imports memory, `realloc`,
//!    `reset`, and the lowered host `get-environment`. Each exported function calls `reset` at
//!    the top so that every call starts with a clean heap — safe because each call is
//!    independent and the component model guarantees that memory contents aren't
//!    observable between calls.
//!
//! # Canonical ABI layout
//!
//! The canonical ABI representation of `get-environment: func() -> list<tuple<string, string>>`
//! when lowered to a core function uses:
//! - A return pointer parameter (i32) pointing to where the result is stored
//! - The result at the return pointer is: (i32, i32) = (list_ptr, list_len)
//! - Each list element is a tuple of two strings: (ptr, len, ptr, len) = 4 × i32 = 16 bytes

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataCountSection, DataSection, DataSegment, DataSegmentMode,
    EntityType, ExportKind, ExportSection, Function, FunctionSection, GlobalSection, GlobalType,
    ImportSection, MemArg, MemorySection, MemoryType, Module, TypeSection, ValType,
};

// --- Filter module indices ---

// Type indices
const FILTER_TY_LOWERED: u32 = 0; // (i32) -> ()
const FILTER_TY_REALLOC: u32 = 1; // (i32, i32, i32, i32) -> i32
const FILTER_TY_LIFTED: u32 = 2; // () -> (i32)
const FILTER_TY_RESET: u32 = 3; // () -> ()

// Imported function indices (imports precede defined functions in Wasm)
const FILTER_FN_GET_ENVIRONMENT: u32 = 0;
const FILTER_FN_REALLOC: u32 = 1;
const FILTER_FN_RESET: u32 = 2;

// Defined function indices
const FILTER_FN_GET_ENV_BASE: u32 = 3;

// --- Memory module indices ---

// Type indices
const MEM_TY_REALLOC: u32 = 0;
const MEM_TY_RESET: u32 = 1;

// Function indices
const MEM_FN_REALLOC: u32 = 0;
const MEM_FN_RESET: u32 = 1;

// Global indices
const MEM_GLOBAL_BUMP_PTR: u32 = 0;

/// Generate a filter core module that imports memory instead of defining its own.
///
/// Imports are:
/// - `"memory"`: linear memory
/// - `"get-environment"`: lowered (i32) -> () [Return Pointer]
/// - `"realloc"`: (i32, i32, i32, i32) -> i32
/// - `"reset"`: () -> () — resets the bump allocator (called at the start of each export)
pub fn generate_env_filter_module(prefixes: &[&str]) -> Vec<u8> {
    let mut module = Module::new();

    // === Type section ===
    // Types are assigned indices sequentially; see FILTER_TY_* constants.
    let mut types = TypeSection::new();
    types.ty().function(vec![ValType::I32], vec![]);
    types
        .ty()
        .function(vec![ValType::I32; 4], vec![ValType::I32]);
    types.ty().function(vec![], vec![ValType::I32]);
    types.ty().function(vec![], vec![]);
    module.section(&types);

    // === Import section ===
    let mut imports = ImportSection::new();
    imports.import(
        "host",
        "memory",
        EntityType::Memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        }),
    );
    imports.import(
        "host",
        "get-environment",
        EntityType::Function(FILTER_TY_LOWERED),
    );
    imports.import("host", "realloc", EntityType::Function(FILTER_TY_REALLOC));
    imports.import("host", "reset", EntityType::Function(FILTER_TY_RESET));
    module.section(&imports);

    // === Function section ===
    let num_prefix_funcs = prefixes.len() as u32;
    let mut functions = FunctionSection::new();
    for _ in 0..num_prefix_funcs {
        functions.function(FILTER_TY_LIFTED);
    }
    module.section(&functions);

    // === Export section ===
    let mut exports = ExportSection::new();
    for i in 0..num_prefix_funcs {
        exports.export(
            &format!("get-environment-{i}"),
            ExportKind::Func,
            FILTER_FN_GET_ENV_BASE + i,
        );
    }
    module.section(&exports);

    module.section(&DataCountSection {
        count: prefixes.len() as u32,
    });

    // === Code section ===
    let mut codes = CodeSection::new();

    // Filtered get-environment for each prefix
    for (i, prefix) in prefixes.iter().enumerate() {
        let prefix_offset = compute_prefix_offset(prefixes, i);
        let prefix_len = prefix.len() as i32;
        codes.function(&build_filter_env_function(prefix_offset, prefix_len));
    }

    module.section(&codes);

    // === Data section ===
    let mut data = DataSection::new();
    let mut offset = 0i32;
    for prefix in prefixes {
        data.segment(DataSegment {
            mode: DataSegmentMode::Active {
                memory_index: 0,
                offset: &ConstExpr::i32_const(offset),
            },
            data: prefix.as_bytes().to_vec(),
        });
        offset += prefix.len() as i32;
    }
    module.section(&data);

    module.finish()
}

/// Build a minimal core module that provides memory, a bump-allocator `realloc`,
/// and a `reset` function.
///
/// The `heap_start` parameter is the byte offset into the module's linear memory at which
/// the dynamic heap begins. Callers should set this to the end of all static data segments
/// (and any required alignment padding) so that the bump-allocator does not overlap or
/// overwrite embedded static data.
///
/// # Exports
///
/// - `memory`: the linear memory (1 page minimum, growable)
/// - `realloc(old_ptr, old_size, align, new_size) -> ptr`: bump allocator that grows
///   memory automatically when needed
/// - `reset()`: resets the bump pointer back to `heap_start`, effectively freeing all
///   dynamic allocations. Safe because each component-model call is independent.
pub fn build_memory_module(heap_start: u32) -> Module {
    let mut module = Module::new();

    // Type section — indices must match MEM_TY_* constants
    let mut types = TypeSection::new();
    types
        .ty()
        .function(vec![ValType::I32; 4], vec![ValType::I32]);
    types.ty().function(vec![], vec![]);
    module.section(&types);

    // Function section
    let mut functions = FunctionSection::new();
    functions.function(MEM_TY_REALLOC);
    functions.function(MEM_TY_RESET);
    module.section(&functions);

    // Memory section
    let mut memories = MemorySection::new();
    memories.memory(MemoryType {
        minimum: 1,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    module.section(&memories);

    // Global section — bump pointer
    let mut globals = GlobalSection::new();
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(heap_start as i32),
    );
    module.section(&globals);

    // Export section
    let mut exports = ExportSection::new();
    exports.export("memory", ExportKind::Memory, 0);
    exports.export("realloc", ExportKind::Func, MEM_FN_REALLOC);
    exports.export("reset", ExportKind::Func, MEM_FN_RESET);
    module.section(&exports);

    // Code section
    let mut codes = CodeSection::new();

    // realloc — bump allocator with memory growth
    //
    // Pseudocode:
    //   aligned = (bump_ptr + align - 1) & ~(align - 1)
    //   new_bump = aligned + new_size
    //   if new_bump > memory.size * 65536:
    //       pages_needed = ceil((new_bump - mem_bytes) / 65536)
    //       if memory.grow(pages_needed) == -1: unreachable
    //   bump_ptr = new_bump
    //   return aligned
    {
        // params: 0=old_ptr, 1=old_size, 2=align, 3=new_size
        // locals: 4=aligned_ptr, 5=new_bump
        let mut f = Function::new(vec![(2, ValType::I32)]);

        f.instructions()
            // aligned = (bump_ptr + align - 1) & ~(align - 1)
            .global_get(MEM_GLOBAL_BUMP_PTR)
            .local_get(2)
            .i32_const(1)
            .i32_sub()
            .i32_add()
            .i32_const(0)
            .local_get(2)
            .i32_sub()
            .i32_and()
            .local_set(4)
            // new_bump = aligned + new_size
            .local_get(4)
            .local_get(3)
            .i32_add()
            .local_set(5)
            // if new_bump > memory.size * 65536, grow
            .local_get(5)
            .memory_size(0)
            .i32_const(65536)
            .i32_mul()
            .i32_gt_u()
            .if_(BlockType::Empty)
            // pages_needed = ceil((new_bump - mem_bytes) / 65536)
            .local_get(5)
            .memory_size(0)
            .i32_const(65536)
            .i32_mul()
            .i32_sub()
            .i32_const(65535)
            .i32_add()
            .i32_const(65536)
            .i32_div_u()
            .memory_grow(0)
            // memory.grow returns -1 on failure
            .i32_const(-1)
            .i32_eq()
            .if_(BlockType::Empty)
            .unreachable()
            .end()
            .end()
            // bump_ptr = new_bump
            .local_get(5)
            .global_set(MEM_GLOBAL_BUMP_PTR)
            // return aligned
            .local_get(4)
            .end();

        codes.function(&f);
    }

    // reset — set bump pointer back to heap_start
    {
        let mut f = Function::new(vec![]);
        f.instructions()
            .i32_const(heap_start as i32)
            .global_set(MEM_GLOBAL_BUMP_PTR)
            .end();
        codes.function(&f);
    }

    module.section(&codes);

    module
}

/// Compute the memory offset of the i-th prefix string.
fn compute_prefix_offset(prefixes: &[&str], index: usize) -> i32 {
    prefixes[..index].iter().map(|p| p.len() as i32).sum()
}

/// Build the filter function for a single component's get-environment.
///
/// This function:
/// 1. Calls `reset` to reclaim all prior allocations
/// 2. Calls the host's get-environment (lowered, return-pointer) to get all env vars
/// 3. Iterates through them, checking if each key starts with the prefix
/// 4. Builds a new list with matching entries (prefix stripped from key)
///
/// The function signature is `() -> (i32)` (spilled return):
/// the function allocates a result area, writes `(list_ptr, list_len)` to it,
/// and returns a pointer to that area.
///
fn build_filter_env_function(prefix_offset: i32, prefix_len: i32) -> Function {
    // No params, locals start at index 0
    const TEMP_PTR: u32 = 0;
    const HOST_LIST_PTR: u32 = 1;
    const HOST_LIST_LEN: u32 = 2;
    const OUTPUT_LIST: u32 = 3;
    const OUTPUT_COUNT: u32 = 4;
    const ELEMENT_PTR: u32 = 5;
    const KEY_PTR: u32 = 6;
    const KEY_LEN: u32 = 7;
    const VAL_PTR: u32 = 8;
    const VAL_LEN: u32 = 9;
    const MATCH_FLAG: u32 = 10;
    const LOOP_I: u32 = 11;
    const LOOP_J: u32 = 12;
    const DEST_BASE: u32 = 13;

    let locals = vec![(14, ValType::I32)];
    let mut f = Function::new(locals);

    let mut insn = f.instructions();

    // Reset bump allocator — safe because each call is independent
    insn.call(FILTER_FN_RESET);

    // Allocate 8 bytes for temp return area to call host
    insn.i32_const(0)
        .i32_const(0)
        .i32_const(4)
        .i32_const(8)
        .call(FILTER_FN_REALLOC)
        .local_tee(TEMP_PTR)
        .call(FILTER_FN_GET_ENVIRONMENT);

    // Read host list pointer and length from temp area
    insn.local_get(TEMP_PTR)
        .i32_load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        })
        .local_set(HOST_LIST_PTR);

    insn.local_get(TEMP_PTR)
        .i32_load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        })
        .local_set(HOST_LIST_LEN);

    // Allocate output list (worst case: host_list_len entries x 16 bytes each)
    insn.i32_const(0)
        .i32_const(0)
        .i32_const(4)
        .local_get(HOST_LIST_LEN)
        .i32_const(16)
        .i32_mul()
        .call(FILTER_FN_REALLOC)
        .local_set(OUTPUT_LIST);

    // Initialize counters
    insn.i32_const(0).local_set(OUTPUT_COUNT);
    insn.i32_const(0).local_set(LOOP_I);

    // Loop over all env vars
    insn.block(BlockType::Empty);
    insn.loop_(BlockType::Empty);

    // if i >= host_list_len, break
    insn.local_get(LOOP_I)
        .local_get(HOST_LIST_LEN)
        .i32_ge_u()
        .br_if(1);

    // element_ptr = host_list_ptr + i * 16
    insn.local_get(HOST_LIST_PTR)
        .local_get(LOOP_I)
        .i32_const(16)
        .i32_mul()
        .i32_add()
        .local_set(ELEMENT_PTR);

    // Read key_ptr from element
    insn.local_get(ELEMENT_PTR)
        .i32_load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        })
        .local_set(KEY_PTR);

    // Read key_len from element
    insn.local_get(ELEMENT_PTR)
        .i32_load(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        })
        .local_set(KEY_LEN);

    // Read val_ptr from element
    insn.local_get(ELEMENT_PTR)
        .i32_load(MemArg {
            offset: 8,
            align: 2,
            memory_index: 0,
        })
        .local_set(VAL_PTR);

    // Read val_len from element
    insn.local_get(ELEMENT_PTR)
        .i32_load(MemArg {
            offset: 12,
            align: 2,
            memory_index: 0,
        })
        .local_set(VAL_LEN);

    // Check if key_len >= prefix_len
    insn.local_get(KEY_LEN).i32_const(prefix_len).i32_lt_u();

    // If key is shorter than prefix, skip
    insn.if_(BlockType::Empty);
    {
        insn.local_get(LOOP_I)
            .i32_const(1)
            .i32_add()
            .local_set(LOOP_I);
        insn.br(1);
    }
    insn.end();

    // Compare prefix bytes
    insn.i32_const(1).local_set(MATCH_FLAG);
    insn.i32_const(0).local_set(LOOP_J);

    insn.block(BlockType::Empty);
    insn.loop_(BlockType::Empty);

    // if j >= prefix_len, break
    insn.local_get(LOOP_J)
        .i32_const(prefix_len)
        .i32_ge_u()
        .br_if(1);

    // compare key[j] vs prefix[j]
    insn.local_get(KEY_PTR)
        .local_get(LOOP_J)
        .i32_add()
        .i32_load8_u(MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        });

    insn.i32_const(prefix_offset)
        .local_get(LOOP_J)
        .i32_add()
        .i32_load8_u(MemArg {
            offset: 0,
            align: 0,
            memory_index: 0,
        });

    insn.i32_ne();

    insn.if_(BlockType::Empty);
    {
        insn.i32_const(0).local_set(MATCH_FLAG);
        insn.br(2);
    }
    insn.end();

    // j++
    insn.local_get(LOOP_J)
        .i32_const(1)
        .i32_add()
        .local_set(LOOP_J);

    insn.br(0);
    insn.end(); // loop
    insn.end(); // block

    // If no match, skip
    insn.local_get(MATCH_FLAG).i32_eqz();

    insn.if_(BlockType::Empty);
    {
        insn.local_get(LOOP_I)
            .i32_const(1)
            .i32_add()
            .local_set(LOOP_I);
        insn.br(1);
    }
    insn.end();

    // Matched — write to output list
    // dest = output_list + output_count * 16
    insn.local_get(OUTPUT_LIST)
        .local_get(OUTPUT_COUNT)
        .i32_const(16)
        .i32_mul()
        .i32_add()
        .local_set(DEST_BASE);

    insn.local_get(DEST_BASE);
    insn.local_get(KEY_PTR).i32_const(prefix_len).i32_add();
    insn.i32_store(MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    });

    insn.local_get(DEST_BASE);
    insn.local_get(KEY_LEN).i32_const(prefix_len).i32_sub();
    insn.i32_store(MemArg {
        offset: 4,
        align: 2,
        memory_index: 0,
    });

    insn.local_get(DEST_BASE);
    insn.local_get(VAL_PTR);
    insn.i32_store(MemArg {
        offset: 8,
        align: 2,
        memory_index: 0,
    });

    insn.local_get(DEST_BASE);
    insn.local_get(VAL_LEN);
    insn.i32_store(MemArg {
        offset: 12,
        align: 2,
        memory_index: 0,
    });

    // output_count++
    insn.local_get(OUTPUT_COUNT)
        .i32_const(1)
        .i32_add()
        .local_set(OUTPUT_COUNT);

    // i++
    insn.local_get(LOOP_I)
        .i32_const(1)
        .i32_add()
        .local_set(LOOP_I);

    insn.br(0);
    insn.end(); // loop
    insn.end(); // block

    // Spilled return: allocate result area, write to it, return pointer.
    insn.i32_const(0)
        .i32_const(0)
        .i32_const(4)
        .i32_const(8)
        .call(FILTER_FN_REALLOC)
        .local_set(TEMP_PTR);
    insn.local_get(TEMP_PTR)
        .local_get(OUTPUT_LIST)
        .i32_store(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        });
    insn.local_get(TEMP_PTR)
        .local_get(OUTPUT_COUNT)
        .i32_store(MemArg {
            offset: 4,
            align: 2,
            memory_index: 0,
        });
    insn.local_get(TEMP_PTR);

    insn.end();

    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_validates() {
        let module_bytes = generate_env_filter_module(&["main_", "sub_"]);
        wasmparser::validate(&module_bytes).expect("generated module should be valid");
    }

    #[test]
    fn test_generate_single_prefix() {
        let module_bytes = generate_env_filter_module(&["app_"]);
        wasmparser::validate(&module_bytes).expect("single prefix module should be valid");
    }

    #[test]
    fn test_generate_many_prefixes() {
        let owned_prefixes: Vec<String> = (0..10).map(|i| format!("comp{i}_")).collect();
        let prefixes: Vec<&str> = owned_prefixes.iter().map(|s| s.as_str()).collect();
        let module_bytes = generate_env_filter_module(&prefixes);
        wasmparser::validate(&module_bytes).expect("many-prefix module should be valid");
    }

    // --- wasmtime runtime tests for the memory module ---

    /// Helper: instantiate the memory module in wasmtime and return (store, instance, memory).
    fn instantiate_memory_module(
        heap_start: u32,
    ) -> (wasmtime::Store<()>, wasmtime::Instance, wasmtime::Memory) {
        let module = build_memory_module(heap_start);
        let bytes = module.finish();
        wasmparser::validate(&bytes).expect("memory module should be valid");

        let engine = wasmtime::Engine::default();
        let wasm_module =
            wasmtime::Module::new(&engine, &bytes).expect("failed to compile memory module");
        let mut store = wasmtime::Store::new(&engine, ());
        let instance = wasmtime::Instance::new(&mut store, &wasm_module, &[])
            .expect("failed to instantiate memory module");
        let memory = instance
            .get_memory(&mut store, "memory")
            .expect("missing memory export");
        (store, instance, memory)
    }

    /// Call the exported `realloc` function.
    fn call_realloc(
        store: &mut wasmtime::Store<()>,
        instance: &wasmtime::Instance,
        old_ptr: i32,
        old_size: i32,
        align: i32,
        new_size: i32,
    ) -> i32 {
        let realloc = instance
            .get_typed_func::<(i32, i32, i32, i32), i32>(&mut *store, "realloc")
            .expect("missing realloc export");
        realloc
            .call(&mut *store, (old_ptr, old_size, align, new_size))
            .expect("realloc call failed")
    }

    /// Call the exported `reset` function.
    fn call_reset(store: &mut wasmtime::Store<()>, instance: &wasmtime::Instance) {
        let reset = instance
            .get_typed_func::<(), ()>(&mut *store, "reset")
            .expect("missing reset export");
        reset.call(&mut *store, ()).expect("reset call failed");
    }

    #[test]
    fn realloc_returns_aligned_pointers() {
        let (mut store, instance, _memory) = instantiate_memory_module(0);

        // Allocate with alignment 4
        let ptr1 = call_realloc(&mut store, &instance, 0, 0, 4, 10);
        assert_eq!(ptr1 % 4, 0, "pointer should be 4-byte aligned");

        // Allocate with alignment 8
        let ptr2 = call_realloc(&mut store, &instance, 0, 0, 8, 20);
        assert_eq!(ptr2 % 8, 0, "pointer should be 8-byte aligned");
        assert!(
            ptr2 >= ptr1 + 10,
            "second allocation should not overlap first"
        );
    }

    #[test]
    fn realloc_respects_heap_start() {
        let heap_start = 128u32;
        let (mut store, instance, _memory) = instantiate_memory_module(heap_start);

        let ptr = call_realloc(&mut store, &instance, 0, 0, 1, 8);
        assert!(
            ptr >= heap_start as i32,
            "allocation should be at or after heap_start ({ptr} < {heap_start})"
        );
    }

    #[test]
    fn reset_reclaims_memory() {
        let heap_start = 64u32;
        let (mut store, instance, _memory) = instantiate_memory_module(heap_start);

        let ptr1 = call_realloc(&mut store, &instance, 0, 0, 4, 100);
        let ptr2 = call_realloc(&mut store, &instance, 0, 0, 4, 100);
        assert!(ptr2 > ptr1, "second alloc should be after first");

        // After reset, the next allocation should start back at heap_start
        call_reset(&mut store, &instance);
        let ptr3 = call_realloc(&mut store, &instance, 0, 0, 4, 100);
        assert_eq!(
            ptr3, ptr1,
            "after reset, allocation should reuse the same address"
        );
    }

    #[test]
    fn realloc_grows_memory_when_needed() {
        // Start with heap_start near the end of the first page (64KiB = 65536)
        let heap_start = 65000u32;
        let (mut store, instance, memory) = instantiate_memory_module(heap_start);

        let initial_pages = memory.size(&store);
        assert_eq!(initial_pages, 1);

        // Allocate more than the remaining space in the first page
        let ptr = call_realloc(&mut store, &instance, 0, 0, 4, 2000);
        assert!(ptr >= heap_start as i32);

        let new_pages = memory.size(&store);
        assert!(
            new_pages > initial_pages,
            "memory should have grown (was {initial_pages}, now {new_pages})"
        );
    }

    #[test]
    fn realloc_handles_large_allocation() {
        let (mut store, instance, memory) = instantiate_memory_module(0);

        // Allocate 3 full pages worth of data (196608 bytes)
        let size = 3 * 65536;
        let ptr = call_realloc(&mut store, &instance, 0, 0, 1, size);
        assert_eq!(ptr, 0);

        let new_pages = memory.size(&store);
        // Started with 1 page (65536 bytes), need 196608 total → grow by 2 → 3 pages
        assert!(
            new_pages >= 3,
            "memory should be at least 3 pages, got {new_pages}"
        );
    }

    #[test]
    fn repeated_calls_with_reset_dont_grow_unboundedly() {
        let (mut store, instance, memory) = instantiate_memory_module(0);

        // Simulate many calls, each allocating ~1000 bytes then resetting
        for _ in 0..1000 {
            call_realloc(&mut store, &instance, 0, 0, 4, 1000);
            call_reset(&mut store, &instance);
        }

        // Memory should not have grown beyond 1 page since we reset each time
        let pages = memory.size(&store);
        assert_eq!(
            pages, 1,
            "memory should still be 1 page after repeated alloc+reset cycles"
        );
    }

    #[test]
    fn realloc_data_is_writable_and_readable() {
        let (mut store, instance, memory) = instantiate_memory_module(0);

        let ptr = call_realloc(&mut store, &instance, 0, 0, 1, 4) as usize;
        // Write and read back data
        memory.data_mut(&mut store)[ptr..ptr + 4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        let read = &memory.data(&store)[ptr..ptr + 4];
        assert_eq!(read, &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    // --- Integrated filter + memory module tests ---
    //
    // These tests link the filter module with the memory module and mock host
    // functions to verify end-to-end filtering behavior at the core Wasm level.

    /// Environment variables to inject via the mock host get-environment.
    /// Stored as a Vec to be shared with the host callback via store data.
    struct FilterTestState {
        env_vars: Vec<(String, String)>,
    }

    /// Write a string into linear memory at `offset`, returning the number of bytes written.
    fn write_str(
        memory: &wasmtime::Memory,
        store: &mut impl wasmtime::AsContextMut,
        offset: usize,
        s: &str,
    ) -> usize {
        let bytes = s.as_bytes();
        memory.data_mut(store)[offset..offset + bytes.len()].copy_from_slice(bytes);
        bytes.len()
    }

    /// Read (list_ptr, list_len) from the return area, then read each
    /// (key_ptr, key_len, val_ptr, val_len) tuple from the list.
    fn read_env_result(
        memory: &wasmtime::Memory,
        store: &wasmtime::Store<FilterTestState>,
        result_ptr: i32,
    ) -> Vec<(String, String)> {
        let data = memory.data(store);
        let rp = result_ptr as usize;
        let list_ptr = u32::from_le_bytes(data[rp..rp + 4].try_into().unwrap()) as usize;
        let list_len = u32::from_le_bytes(data[rp + 4..rp + 8].try_into().unwrap()) as usize;

        let mut result = Vec::with_capacity(list_len);
        for i in 0..list_len {
            let base = list_ptr + i * 16;
            let kp = u32::from_le_bytes(data[base..base + 4].try_into().unwrap()) as usize;
            let kl = u32::from_le_bytes(data[base + 4..base + 8].try_into().unwrap()) as usize;
            let vp = u32::from_le_bytes(data[base + 8..base + 12].try_into().unwrap()) as usize;
            let vl = u32::from_le_bytes(data[base + 12..base + 16].try_into().unwrap()) as usize;
            let key = std::str::from_utf8(&data[kp..kp + kl]).unwrap().to_string();
            let val = std::str::from_utf8(&data[vp..vp + vl]).unwrap().to_string();
            result.push((key, val));
        }
        result
    }

    /// Instantiate the filter module linked with the memory module and a mock
    /// `get-environment` host function that writes `env_vars` into linear memory.
    fn instantiate_filter_module(
        prefixes: &[&str],
        env_vars: Vec<(String, String)>,
    ) -> (
        wasmtime::Store<FilterTestState>,
        wasmtime::Instance,
        wasmtime::Memory,
    ) {
        let total_prefix_bytes: usize = prefixes.iter().map(|p| p.len()).sum();

        // Build both modules
        let mem_module = build_memory_module(total_prefix_bytes as u32);
        let mem_bytes = mem_module.finish();
        let filter_bytes = generate_env_filter_module(prefixes);

        let engine = wasmtime::Engine::default();
        let mem_wasm = wasmtime::Module::new(&engine, &mem_bytes).unwrap();
        let filter_wasm = wasmtime::Module::new(&engine, &filter_bytes).unwrap();

        let mut store = wasmtime::Store::new(&engine, FilterTestState { env_vars });

        // Instantiate memory module (no imports)
        let mem_instance = wasmtime::Instance::new(&mut store, &mem_wasm, &[]).unwrap();
        let memory = mem_instance.get_memory(&mut store, "memory").unwrap();
        let realloc_fn = mem_instance.get_func(&mut store, "realloc").unwrap();
        let reset_fn = mem_instance.get_func(&mut store, "reset").unwrap();

        // Create mock host functions for get-environment, get-arguments, initial-cwd.
        //
        // get-environment writes the env vars into linear memory using realloc
        // for allocations, then writes (list_ptr, list_len) at the return pointer.
        let get_env = wasmtime::Func::wrap(
            &mut store,
            move |mut caller: wasmtime::Caller<'_, FilterTestState>, ret_ptr: i32| {
                let env_vars = caller.data().env_vars.clone();
                let n = env_vars.len();

                // Allocate list: n entries × 16 bytes
                let list_size = (n * 16) as i32;
                let mut results = [wasmtime::Val::I32(0)];
                realloc_fn
                    .call(
                        &mut caller,
                        &[
                            wasmtime::Val::I32(0),
                            wasmtime::Val::I32(0),
                            wasmtime::Val::I32(4),
                            wasmtime::Val::I32(list_size),
                        ],
                        &mut results,
                    )
                    .unwrap();
                let list_ptr = results[0].unwrap_i32();

                // For each env var, allocate and write key and value strings
                for (i, (key, val)) in env_vars.iter().enumerate() {
                    // Allocate key
                    let mut kr = [wasmtime::Val::I32(0)];
                    realloc_fn
                        .call(
                            &mut caller,
                            &[
                                wasmtime::Val::I32(0),
                                wasmtime::Val::I32(0),
                                wasmtime::Val::I32(1),
                                wasmtime::Val::I32(key.len() as i32),
                            ],
                            &mut kr,
                        )
                        .unwrap();
                    let key_ptr = kr[0].unwrap_i32();
                    write_str(&memory, &mut caller, key_ptr as usize, key);

                    // Allocate val
                    let mut vr = [wasmtime::Val::I32(0)];
                    realloc_fn
                        .call(
                            &mut caller,
                            &[
                                wasmtime::Val::I32(0),
                                wasmtime::Val::I32(0),
                                wasmtime::Val::I32(1),
                                wasmtime::Val::I32(val.len() as i32),
                            ],
                            &mut vr,
                        )
                        .unwrap();
                    let val_ptr = vr[0].unwrap_i32();
                    write_str(&memory, &mut caller, val_ptr as usize, val);

                    // Write tuple into list
                    let base = (list_ptr + (i as i32) * 16) as usize;
                    let data = memory.data_mut(&mut caller);
                    data[base..base + 4].copy_from_slice(&(key_ptr as u32).to_le_bytes());
                    data[base + 4..base + 8].copy_from_slice(&(key.len() as u32).to_le_bytes());
                    data[base + 8..base + 12].copy_from_slice(&(val_ptr as u32).to_le_bytes());
                    data[base + 12..base + 16].copy_from_slice(&(val.len() as u32).to_le_bytes());
                }

                // Write (list_ptr, list_len) at ret_ptr
                let rp = ret_ptr as usize;
                let data = memory.data_mut(&mut caller);
                data[rp..rp + 4].copy_from_slice(&(list_ptr as u32).to_le_bytes());
                data[rp + 4..rp + 8].copy_from_slice(&(n as u32).to_le_bytes());
            },
        );

        // Instantiate filter module with imports
        let filter_instance = wasmtime::Instance::new(
            &mut store,
            &filter_wasm,
            &[
                memory.into(),
                get_env.into(),
                realloc_fn.into(),
                reset_fn.into(),
            ],
        )
        .unwrap();

        (store, filter_instance, memory)
    }

    #[test]
    fn filter_basic_prefix_matching() {
        let env_vars = vec![
            ("APP_FOO".into(), "val1".into()),
            ("APP_BAR".into(), "val2".into()),
            ("OTHER_X".into(), "val3".into()),
        ];
        let (mut store, instance, memory) = instantiate_filter_module(&["APP_"], env_vars);

        let get_env_0 = instance
            .get_typed_func::<(), i32>(&mut store, "get-environment-0")
            .unwrap();
        let result_ptr = get_env_0.call(&mut store, ()).unwrap();
        let vars = read_env_result(&memory, &store, result_ptr);

        // Should contain FOO and BAR (prefix "APP_" stripped)
        assert_eq!(vars.len(), 2);
        assert!(vars.contains(&("FOO".to_string(), "val1".to_string())));
        assert!(vars.contains(&("BAR".to_string(), "val2".to_string())));
    }

    #[test]
    fn filter_no_matches_returns_empty() {
        let env_vars = vec![
            ("OTHER_X".into(), "val1".into()),
            ("THING_Y".into(), "val2".into()),
        ];
        let (mut store, instance, memory) = instantiate_filter_module(&["APP_"], env_vars);

        let get_env_0 = instance
            .get_typed_func::<(), i32>(&mut store, "get-environment-0")
            .unwrap();
        let result_ptr = get_env_0.call(&mut store, ()).unwrap();
        let vars = read_env_result(&memory, &store, result_ptr);

        assert!(vars.is_empty());
    }

    #[test]
    fn filter_multiple_prefixes() {
        let env_vars = vec![
            ("MAIN_A".into(), "1".into()),
            ("MAIN_B".into(), "2".into()),
            ("SUB_C".into(), "3".into()),
            ("SUB_D".into(), "4".into()),
            ("OTHER".into(), "5".into()),
        ];
        let (mut store, instance, memory) = instantiate_filter_module(&["MAIN_", "SUB_"], env_vars);

        // Check MAIN_ filter
        let get_env_0 = instance
            .get_typed_func::<(), i32>(&mut store, "get-environment-0")
            .unwrap();
        let result_ptr = get_env_0.call(&mut store, ()).unwrap();
        let main_vars = read_env_result(&memory, &store, result_ptr);
        assert_eq!(main_vars.len(), 2);
        assert!(main_vars.contains(&("A".to_string(), "1".to_string())));
        assert!(main_vars.contains(&("B".to_string(), "2".to_string())));

        // Check SUB_ filter
        let get_env_1 = instance
            .get_typed_func::<(), i32>(&mut store, "get-environment-1")
            .unwrap();
        let result_ptr = get_env_1.call(&mut store, ()).unwrap();
        let sub_vars = read_env_result(&memory, &store, result_ptr);
        assert_eq!(sub_vars.len(), 2);
        assert!(sub_vars.contains(&("C".to_string(), "3".to_string())));
        assert!(sub_vars.contains(&("D".to_string(), "4".to_string())));
    }

    #[test]
    fn filter_repeated_calls_dont_leak_memory() {
        let env_vars = vec![("APP_X".into(), "value".into())];
        let (mut store, instance, memory) = instantiate_filter_module(&["APP_"], env_vars);

        let get_env_0 = instance
            .get_typed_func::<(), i32>(&mut store, "get-environment-0")
            .unwrap();

        // Call many times — the reset at the top of each call prevents unbounded growth
        for _ in 0..500 {
            let result_ptr = get_env_0.call(&mut store, ()).unwrap();
            let vars = read_env_result(&memory, &store, result_ptr);
            assert_eq!(vars.len(), 1);
            assert_eq!(vars[0], ("X".to_string(), "value".to_string()));
        }

        // Memory should not have grown beyond 1 page for this small dataset
        let pages = memory.size(&store);
        assert_eq!(
            pages, 1,
            "memory should still be 1 page after repeated calls"
        );
    }

    #[test]
    fn filter_exact_prefix_match_yields_empty_key() {
        // A key that exactly equals the prefix should produce an empty key
        let env_vars = vec![("PRE_".into(), "val".into())];
        let (mut store, instance, memory) = instantiate_filter_module(&["PRE_"], env_vars);

        let get_env_0 = instance
            .get_typed_func::<(), i32>(&mut store, "get-environment-0")
            .unwrap();
        let result_ptr = get_env_0.call(&mut store, ()).unwrap();
        let vars = read_env_result(&memory, &store, result_ptr);

        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0], ("".to_string(), "val".to_string()));
    }

    #[test]
    fn filter_key_shorter_than_prefix_is_skipped() {
        let env_vars = vec![
            ("AB".into(), "short".into()),      // shorter than prefix "ABCDE_"
            ("ABCDE_X".into(), "match".into()), // matches
        ];
        let (mut store, instance, memory) = instantiate_filter_module(&["ABCDE_"], env_vars);

        let get_env_0 = instance
            .get_typed_func::<(), i32>(&mut store, "get-environment-0")
            .unwrap();
        let result_ptr = get_env_0.call(&mut store, ()).unwrap();
        let vars = read_env_result(&memory, &store, result_ptr);

        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0], ("X".to_string(), "match".to_string()));
    }
}
