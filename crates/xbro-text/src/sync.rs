//! Poison-tolerant lock helpers: a panic on another thread must not make the
//! font caches unusable (the cached data stays consistent: every critical
//! section only inserts or clears whole entries).

use std::sync::{Mutex, MutexGuard, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Acquires a read guard, ignoring poisoning.
#[inline]
pub(crate) fn read<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

/// Acquires a write guard, ignoring poisoning.
#[inline]
pub(crate) fn write<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

/// Locks a mutex, ignoring poisoning.
#[inline]
pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn poisoned_locks_still_work() {
        let rw = Arc::new(RwLock::new(1));
        let mutex = Arc::new(Mutex::new(2));
        let (rw2, mutex2) = (Arc::clone(&rw), Arc::clone(&mutex));
        let result = std::thread::spawn(move || {
            let _a = rw2.write().unwrap_or_else(PoisonError::into_inner);
            let _b = mutex2.lock().unwrap_or_else(PoisonError::into_inner);
            panic!("poison");
        })
        .join();
        assert!(result.is_err());
        assert!(rw.is_poisoned() && mutex.is_poisoned());
        assert_eq!(*read(&rw), 1);
        *write(&rw) = 3;
        assert_eq!(*read(&rw), 3);
        assert_eq!(*lock(&mutex), 2);
    }
}
