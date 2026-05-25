use core::any::TypeId;
use core::cell::UnsafeCell;
use core::mem::{self, MaybeUninit};

const MAX_RESOURCES: usize = 8;
const STORAGE_SIZE: usize = 128;

struct Entry {
    type_id: TypeId,
    offset: usize,
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
    /// # Resource lifetime
    ///
    /// Inserted resources are **NOT dropped**: `ResourceRegistry` has no `Drop`
    /// impl. This is intentional for the embedded use-case where resources are
    /// `'static` peripheral handles (e.g. `Twim`, `Saadc`) whose lifetime equals
    /// the device lifetime. Dropping them via `ResourceRegistry` is outside scope
    /// for the current MVP; per-entry `Drop` support is tracked in a dedicated
    /// follow-up issue.
    ///
    /// # Panics
    ///
    /// Panics if a resource of the same `TypeId` has already been inserted, or if
    /// the registry is full (more than `MAX_RESOURCES` entries or `STORAGE_SIZE`
    /// bytes used).
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
        // is at most 8 for all types expected in embedded/no_std use (u64, *const _).
        unsafe {
            let base = (*self.storage.get()).0.as_mut_ptr();
            let dst = base.add(offset) as *mut T;
            core::ptr::write(dst, val);
        }

        self.entries[self.count].write(Entry {
            type_id: TypeId::of::<T>(),
            offset,
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
