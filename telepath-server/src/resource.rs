use core::any::TypeId;
use core::cell::UnsafeCell;
use core::mem::{self, MaybeUninit};

const MAX_RESOURCES: usize = 8;
const STORAGE_SIZE: usize = 128;

struct Entry {
    type_id: TypeId,
    offset: usize,
    drop_fn: unsafe fn(*mut u8),
}

// Guarantees the storage buffer is at least 8-byte aligned so that casting
// `base.add(offset)` to `*mut T` is valid for any T with align_of::<T>() <= 8,
// which covers all primitive and peripheral-handle types used in embedded targets.
#[repr(align(8))]
struct AlignedStorage([MaybeUninit<u8>; STORAGE_SIZE]);

pub struct ResourceRegistry {
    entries: [MaybeUninit<Entry>; MAX_RESOURCES],
    count: usize,
    storage: UnsafeCell<AlignedStorage>,
    used: usize,
}

impl Default for ResourceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// Monomorphised drop shim: restores the concrete type from a raw byte pointer
// and calls its destructor. Stored once per Entry at insert time so the
// registry can drop values without retaining generic type information.
unsafe fn drop_in_place_erased<T>(ptr: *mut u8) {
    // SAFETY: caller guarantees `ptr` was produced by `core::ptr::write::<T>`
    // into aligned storage owned by this registry and is still initialised.
    unsafe { core::ptr::drop_in_place(ptr as *mut T) }
}

impl ResourceRegistry {
    pub const fn new() -> Self {
        Self {
            entries: [const { MaybeUninit::uninit() }; MAX_RESOURCES],
            count: 0,
            storage: UnsafeCell::new(AlignedStorage(
                [const { MaybeUninit::uninit() }; STORAGE_SIZE],
            )),
            used: 0,
        }
    }

    /// Move `val` into the registry, keyed by its `TypeId`.
    ///
    /// Inserted resources are dropped in reverse insertion order when the
    /// registry itself is dropped, matching Rust's standard field/local
    /// destruction order.
    ///
    /// # Alignment
    ///
    /// `T` must have `align_of::<T>() <= 8`. `AlignedStorage` guarantees an
    /// 8-byte-aligned base; types with stricter alignment requirements cannot be
    /// stored safely and will cause a panic at insertion time.
    ///
    /// # Panics
    ///
    /// Panics if a resource of the same `TypeId` has already been inserted, if
    /// the registry is full (more than `MAX_RESOURCES` entries or `STORAGE_SIZE`
    /// bytes used), or if `align_of::<T>() > 8`.
    pub fn insert<T: 'static>(&mut self, val: T) {
        // Duplicate TypeId check — fail-fast so silent shadowing is impossible.
        // This also acts as a runtime backstop for compile-time dedup checks in
        // the proc-macro that may miss type aliases or differently-spelled paths.
        let id = TypeId::of::<T>();
        for i in 0..self.count {
            // SAFETY: entries[0..count] are initialised by insert().
            let entry = unsafe { self.entries[i].assume_init_ref() };
            if entry.type_id == id {
                panic!("duplicate resource type: each resource type may appear at most once");
            }
        }

        let align = mem::align_of::<T>();
        let size = mem::size_of::<T>();

        // AlignedStorage is repr(align(8)); types with stricter alignment would
        // produce mis-aligned pointers. Catch this at insertion time.
        assert!(
            align <= 8,
            "resource type alignment {} exceeds AlignedStorage alignment (8)",
            align,
        );

        let offset = (self.used + align - 1) & !(align - 1);

        assert!(
            offset + size <= STORAGE_SIZE,
            "resource storage full ({} + {} > {})",
            offset,
            size,
            STORAGE_SIZE,
        );
        assert!(
            self.count < MAX_RESOURCES,
            "too many resources (max {})",
            MAX_RESOURCES,
        );

        // SAFETY: `offset` is within the buffer and `offset % align_of::<T>() == 0`.
        // AlignedStorage guarantees the base pointer is 8-byte aligned; T's alignment
        // is at most 8 (asserted above).
        unsafe {
            let base = (*self.storage.get()).0.as_mut_ptr();
            let dst = base.add(offset) as *mut T;
            core::ptr::write(dst, val);
        }

        self.entries[self.count].write(Entry {
            type_id: TypeId::of::<T>(),
            offset,
            drop_fn: drop_in_place_erased::<T>,
        });
        self.count += 1;
        self.used = offset + size;
    }

    /// Look up a resource by type, returning a raw pointer.
    ///
    /// # Safety contract for callers
    ///
    /// The returned pointer is valid for the lifetime of the registry. Creating
    /// a `&mut T` from it is safe provided:
    /// - Each concrete type is registered at most once (enforced by `insert`).
    /// - No two live `&mut` references alias the same entry.
    /// - Dispatch is single-threaded (one shim runs at a time).
    pub fn get_ptr<T: 'static>(&self) -> Option<*mut T> {
        let id = TypeId::of::<T>();
        for i in 0..self.count {
            // SAFETY: entries[0..count] are initialised by insert().
            let entry = unsafe { self.entries[i].assume_init_ref() };
            if entry.type_id == id {
                // SAFETY: offset was recorded by insert(); UnsafeCell allows
                // interior mutation through a shared reference.
                let ptr = unsafe {
                    let base = (*self.storage.get()).0.as_mut_ptr();
                    base.add(entry.offset) as *mut T
                };
                return Some(ptr);
            }
        }
        None
    }
}

impl Drop for ResourceRegistry {
    fn drop(&mut self) {
        // Drop in reverse insertion order — matches Rust's standard destruction
        // order for fields/locals so that resources registered later (which may
        // logically depend on earlier ones) are torn down first.
        for i in (0..self.count).rev() {
            // SAFETY: entries[0..self.count] were all initialised by insert().
            let entry = unsafe { self.entries[i].assume_init_ref() };
            // SAFETY: storage at entry.offset holds an initialised T whose
            // drop_fn was recorded by insert(). The registry has exclusive
            // ownership at drop time — no shim borrow can overlap because
            // `&mut self` is required and dispatch only holds `&self`.
            unsafe {
                let base = (*self.storage.get()).0.as_mut_ptr() as *mut u8;
                (entry.drop_fn)(base.add(entry.offset));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn insert_and_get_ptr() {
        let mut reg = ResourceRegistry::new();
        reg.insert(42u32);
        let ptr = reg.get_ptr::<u32>().expect("u32 must be found");
        assert_eq!(unsafe { *ptr }, 42);
    }

    #[test]
    fn get_ptr_returns_none_for_unregistered() {
        let reg = ResourceRegistry::new();
        assert!(reg.get_ptr::<u64>().is_none());
    }

    #[test]
    fn mutation_through_ptr() {
        let mut reg = ResourceRegistry::new();
        reg.insert(0u32);
        let ptr = reg.get_ptr::<u32>().unwrap();
        unsafe { *ptr = 99 };
        let ptr2 = reg.get_ptr::<u32>().unwrap();
        assert_eq!(unsafe { *ptr2 }, 99);
    }

    #[test]
    fn multiple_types() {
        let mut reg = ResourceRegistry::new();
        reg.insert(1u8);
        reg.insert(2u16);
        reg.insert(3u32);
        assert_eq!(unsafe { *reg.get_ptr::<u8>().unwrap() }, 1);
        assert_eq!(unsafe { *reg.get_ptr::<u16>().unwrap() }, 2);
        assert_eq!(unsafe { *reg.get_ptr::<u32>().unwrap() }, 3);
    }

    #[test]
    fn alignment_respected() {
        let mut reg = ResourceRegistry::new();
        reg.insert(1u8);
        reg.insert(2u64); // must be 8-byte aligned despite u8 before it
        let ptr = reg.get_ptr::<u64>().unwrap();
        assert_eq!(ptr as usize % mem::align_of::<u64>(), 0);
        assert_eq!(unsafe { *ptr }, 2);
    }

    #[test]
    #[should_panic(expected = "too many resources")]
    fn panics_on_overflow() {
        let mut reg = ResourceRegistry::new();
        reg.insert(0u8);
        reg.insert(0u16);
        reg.insert(0u32);
        reg.insert(0u64);
        reg.insert(0i8);
        reg.insert(0i16);
        reg.insert(0i32);
        reg.insert(0i64);
        reg.insert(0f32); // 9th — should panic
    }

    #[test]
    #[should_panic(expected = "duplicate")]
    fn panics_on_duplicate_typeid() {
        let mut reg = ResourceRegistry::new();
        reg.insert(0u32);
        reg.insert(1u32); // same TypeId — should panic
    }

    // ── Drop tests ────────────────────────────────────────────────────────────

    // Each newtype is a distinct TypeId so all three can coexist in the registry.
    // The Drop impl records the global sequence position into a per-type slot.
    static DROP_SEQ: AtomicUsize = AtomicUsize::new(0);
    static DROP_POS_A: AtomicUsize = AtomicUsize::new(0);
    static DROP_POS_B: AtomicUsize = AtomicUsize::new(0);
    static DROP_POS_C: AtomicUsize = AtomicUsize::new(0);

    struct CounterA;
    struct CounterB;
    struct CounterC;

    impl Drop for CounterA {
        fn drop(&mut self) {
            DROP_POS_A.store(
                DROP_SEQ.fetch_add(1, Ordering::SeqCst) + 1,
                Ordering::SeqCst,
            );
        }
    }
    impl Drop for CounterB {
        fn drop(&mut self) {
            DROP_POS_B.store(
                DROP_SEQ.fetch_add(1, Ordering::SeqCst) + 1,
                Ordering::SeqCst,
            );
        }
    }
    impl Drop for CounterC {
        fn drop(&mut self) {
            DROP_POS_C.store(
                DROP_SEQ.fetch_add(1, Ordering::SeqCst) + 1,
                Ordering::SeqCst,
            );
        }
    }

    #[test]
    fn drops_in_reverse_insertion_order() {
        // Reset shared atomics — tests may run in any order.
        DROP_SEQ.store(0, Ordering::SeqCst);
        DROP_POS_A.store(0, Ordering::SeqCst);
        DROP_POS_B.store(0, Ordering::SeqCst);
        DROP_POS_C.store(0, Ordering::SeqCst);

        {
            let mut reg = ResourceRegistry::new();
            reg.insert(CounterA); // inserted 1st
            reg.insert(CounterB); // inserted 2nd
            reg.insert(CounterC); // inserted 3rd
        } // reg dropped here → C first, then B, then A

        // Sequence positions: C=1, B=2, A=3 (smaller = earlier in drop order)
        let pos_a = DROP_POS_A.load(Ordering::SeqCst);
        let pos_b = DROP_POS_B.load(Ordering::SeqCst);
        let pos_c = DROP_POS_C.load(Ordering::SeqCst);
        assert!(pos_c < pos_b, "C must be dropped before B");
        assert!(pos_b < pos_a, "B must be dropped before A");
    }

    static DROP_RAN: AtomicUsize = AtomicUsize::new(0);

    struct DropWitness;
    impl Drop for DropWitness {
        fn drop(&mut self) {
            DROP_RAN.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn drop_runs_for_single_entry() {
        DROP_RAN.store(0, Ordering::SeqCst);
        {
            let mut reg = ResourceRegistry::new();
            reg.insert(DropWitness);
        }
        assert_eq!(DROP_RAN.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn empty_registry_drop_is_noop() {
        // Must not panic or exhibit UB.
        drop(ResourceRegistry::new());
    }

    #[test]
    #[should_panic(expected = "alignment")]
    fn panics_on_overalign() {
        #[repr(align(16))]
        #[allow(dead_code)]
        struct OverAligned(u64);

        let mut reg = ResourceRegistry::new();
        reg.insert(OverAligned(0));
    }
}
