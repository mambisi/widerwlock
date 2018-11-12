// Copyright 2018 Mohammad Rezaei.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//

//! # Wide (Partitioned) Read Write Lock
//! This crate implements an enterprise-grade read-write lock, typically used for large data structures.
//! The lock is internally 8x partitioned, which allows it to scale much better with a large number of readers,
//! as compared to `std::sync::RwLock`.
//! Even though this is a large lock, it has better performance characteristic for uncontended single reader
//! or single writer lock than `std::sync::RwLock`.
//! The lock uses a contiguous 576 byte heap area to store its state, so it's not a light-weight lock.
//! If you have a complex data structure that holds a GB of data, this would be an appropriate lock.
//!
//! An interesting feature of this lock, beside its performance, is its Read->Write upgrade mechanics. The `ReadGuard` allows an
//! upgrade to a write-lock and informs the user whether the upgrade succeeded atomically or not. This enables
//! the following pattern:
//! - Read to see if the data structure is in a particular state (e.g. contains a value).
//!   - if not, upgrade to a write lock
//!   - if upgrade was not atomic, perform the (potentially expensive) read again
//!   - write to the structure if needed
//!
//! Here is an example:
//! ```
//! # extern crate widerwlock;
//! use widerwlock::*;
//! use std::collections::HashMap;
//! use std::hash::Hash;
//!
//! struct RwMap<K,V> {
//!     map: WideRwLock<HashMap<K,V>>,
//! }
//!
//! impl<K: Eq + Hash,V> RwMap<K, V> {
//!     pub fn insert_if_absent<F>(&self, key: K, v_factory: F)
//!         where F: FnOnce(&K) -> V {
//!         let guard: ReadGuard<_> = self.map.read();
//!         if !guard.contains_key(&key) { // perform the read.
//!
//!             let result: UpgradeResult<_> = guard.upgrade_to_write_lock();
//!             let atomic = result.atomic();
//!             // the atomic return ensures the (potentially) expensive double read
//!             // can be avoided when atomic
//!             let mut write_guard: WriteGuard<_> = result.into_guard();
//!             if !atomic {
//!                 // we have to check again, because another writer may have come in
//!                 if write_guard.contains_key(&key) { return; }
//!             }
//!             let v = v_factory(&key);
//!             write_guard.insert(key, v);
//!         }
//!     }
//! }
//! ```
//!

extern crate parking_lot_core;

use std::{
    alloc::{self, Layout},
    mem, ptr,
};
use std::sync::atomic::*;
use std::thread;
use std::cell::RefCell;

use parking_lot_core::*;
use std::sync::atomic::AtomicBool;
use std::cell::UnsafeCell;
use std::ops::Deref;
use std::ops::DerefMut;
use std::thread::Thread;

/// A multi-reader, single writer lock.
///
/// The implementation has the following properties:
/// - Non-reentrant. Do no attempt to take this lock multiple times in the same thread. It will deadlock.
/// - Writer preferenced: writers will get preference and hold back incoming readers when waiting.
/// - 8x partitioned: allows very fast multi-reader locking.
/// - Uncontended single threaded performance better than `std::sync::Mutex` and `std::sync:RwLock`
/// - No poisoning. It should be possible to wrap this lock with poison support if needed.
///
/// This struct uses a single pointer to point into a 576 byte (64-byte aligned) heap area
///
pub struct WideRwLock<T: ?Sized> {
    ptr: *mut u64,
    data: UnsafeCell<T>,
}

/// returned from `ReadGuard::upgrade_to_write_lock()`
pub enum UpgradeResult<'a, T: ?Sized + 'a> {
    AtomicUpgrade(WriteGuard<'a, T>), NonAtomicUpgrade(WriteGuard<'a, T>)
}

impl<'a, T: ?Sized + 'a> UpgradeResult<'a, T> {
    pub fn atomic(&self) -> bool {
        match self {
            UpgradeResult::AtomicUpgrade(_) => { return true; }
            UpgradeResult::NonAtomicUpgrade(_) => { return false; }
        }
    }

    pub fn into_guard(self) -> WriteGuard<'a, T> {
        match self {
            UpgradeResult::AtomicUpgrade(w) => { return w; }
            UpgradeResult::NonAtomicUpgrade(w) => { return w; }
        }
    }
}

const READER_BITS: u32 = 25;
const READER_MASK: u32 = (1 << READER_BITS) - 1;

const WRITER_MASK: u32 = 1 << (READER_BITS + 1);
const PREPARING_TO_GO_LOCAL: u32 = 1 << (READER_BITS+2);

const GLOBAL_MASK: u32 = 1 << (READER_BITS+4);

const LOCK_POWER: u32 = 3;

const LOCK_COUNT: u32 = 1 << LOCK_POWER;

const LOCK_MASK: u32 = (1 << LOCK_POWER) - 1;

const WRITE_PARK_TOKEN: ParkToken = ParkToken(1);
const READ_PARK_TOKEN: ParkToken = ParkToken(2);

thread_local!(static THREAD_ID: RefCell<u64> = RefCell::new(thread_id_as_u64_init()));

impl<T> WideRwLock<T> {
    /// creates a new lock
    pub fn new(data: T) -> WideRwLock<T> {
        unsafe {
            if mem::size_of::<RawGlobalLock>() > 64 {
                panic!("ThreadId has gotten too fat!");
            }
            let layout = Layout::from_size_align((1+LOCK_COUNT as usize) * 64, 64).unwrap(); //aligned to cache line
            let ptr = alloc::alloc(layout) as *mut u64;
            RawGlobalLock::from_ptr(ptr).init();
            for i in 0..LOCK_COUNT {
                RawLocalLock::new(ptr, i as usize).init();
            }
            WideRwLock { ptr, data: UnsafeCell::new(data) }
        }
    }
}

unsafe impl<T: ?Sized + Send> Send for WideRwLock<T> {}
unsafe impl<T: ?Sized + Send + Sync> Sync for WideRwLock<T> {}

impl<T: ?Sized> WideRwLock<T> {

    /// Obtain a read lock. Multiple simultaneous read locks will be given if there are no writers waiting.
    /// The lock remains locked until the returned RAII guard is dropped.
    #[inline]
    pub fn read(&self) -> ReadGuard<T> {
        let reader_id = reader_index();
        let local_lock = RawLocalLock::new(self.ptr, reader_id);
        if local_lock.try_read_lock_local() {
            return ReadGuard { lock: & self, reader_id};
        }
        let global_sync = RawGlobalLock::from_ptr(self.ptr);
        if !global_sync.try_fast_read_lock() {
            if self.read_lock_slow(reader_id, &local_lock) {
                return ReadGuard {lock: & self, reader_id }
            }
        }
        ReadGuard {lock: & self, reader_id: LOCK_COUNT as usize }
    }

    /// Obtain a write lock. If a writer is waiting, no more read locks will be given until the writer
    /// has been granted access and finished.
    /// The lock remains locked until the returned RAII guard is dropped.
    #[inline]
    pub fn write(&self) -> WriteGuard<T> {
        let global_sync = RawGlobalLock::from_ptr(self.ptr);
        let (must_prep, must_recheck_global) = global_sync.write_lock();
        if must_prep {
            self.prepare_local_write_locks_and_lock_global();
        }
        if must_recheck_global {
            global_sync.wait_for_reader();
        }
        WriteGuard { lock : &self }
    }

    #[inline]
    fn write_unlock(&self) {
        let global_lock = RawGlobalLock::from_ptr(self.ptr);
        global_lock.write_unlock();
    }

    #[inline]
    fn read_unlock(&self) {
        let global_lock = RawGlobalLock::from_ptr(self.ptr);
        global_lock.read_unlock_zero();
    }

    #[inline]
    fn fast_local_read_unlock(&self, reader_id: usize) {
        let lock = RawLocalLock::new(self.ptr, reader_id);
        if lock.read_unlock() {
            lock.unpark_thread();
        }
    }

    fn upgrade_to_write_lock(&self) -> UpgradeResult<T> {
        if RawGlobalLock::from_ptr(self.ptr).try_single_reader_upgrade() {
            return UpgradeResult::AtomicUpgrade(WriteGuard { lock: &self });
        }
        let global_lock = RawGlobalLock::from_ptr(self.ptr);
        global_lock.read_unlock_zero();
        UpgradeResult::NonAtomicUpgrade(self.write())
    }

    fn slow_upgrade(&self, reader_id: usize) -> UpgradeResult<T> {
        self.fast_local_read_unlock(reader_id);
        UpgradeResult::NonAtomicUpgrade(self.write())
    }

    fn read_lock_slow(&self, reader_id: usize, local_lock: &RawLocalLock) -> bool { // always locks, returns true if the lock was locally taken
        let global_sync = RawGlobalLock::from_ptr(self.ptr);
        let (global_locked_needs_prep, local_locked) = global_sync.read_lock(local_lock);
        if global_locked_needs_prep {
            local_lock.read_lock_deglobalize();
            RawLocalLock::new(self.ptr, (reader_id + 1) & LOCK_MASK as usize).try_deglobalize();
            RawLocalLock::new(self.ptr, (reader_id + 2) & LOCK_MASK as usize).try_deglobalize();
            global_sync.read_unlock_prep_local();
            return true;
        }
        local_locked
    }

    fn prepare_local_write_locks_and_lock_global(&self) {
        let mut remaining_pos = -1;
        for i in 0..LOCK_COUNT {
            if !RawLocalLock::new(self.ptr, i as usize).make_global() {
                remaining_pos = i as i32;
            }
        }
        while remaining_pos >= 0 {
            RawLocalLock::new(self.ptr, remaining_pos as usize).wait_for_reader();
            remaining_pos -= 1;
        }
    }
}

/// Returned from the `write()` method and grants read-write access to the enclosed data
pub struct WriteGuard<'a, T: ?Sized + 'a> {
    lock: &'a WideRwLock<T>
}

/// Returned from the `read()` method and grants read-only access to the enclosed data
pub struct ReadGuard<'a, T: ?Sized + 'a> {
    lock: &'a WideRwLock<T>,
    reader_id: usize
}

impl<'a, T: ?Sized + 'a> WriteGuard<'a, T> {
    /// Downgrade the lock to a read lock. Other writers may take this lock in between, that is,
    /// this is not an atomic downgrade.
    pub fn downgrade_to_read_lock(self) -> ReadGuard<'a, T> {
        let lock = self.lock;
        mem::forget(self);
        lock.write_unlock();
        lock.read()
    }
}

impl<'a, T: ?Sized + 'a> ReadGuard<'a, T> {
    /// Upgrade a read lock to a write lock. If the upgrade happened atomically (no other writers
    /// took the lock in the meantime), the return type indicates that.
    pub fn upgrade_to_write_lock(self) -> UpgradeResult<'a, T> {
        let lock = self.lock;
        let reader_id = self.reader_id;
        mem::forget(self);
        if reader_id < LOCK_COUNT as usize {
            lock.slow_upgrade(reader_id)
        } else {
            lock.upgrade_to_write_lock()
        }
    }
}

impl<T: ?Sized> Drop for WideRwLock<T> {
    #[inline]
    fn drop(&mut self) {
        if !self.ptr.is_null() {
            unsafe {
                let layout = Layout::from_size_align(9 * 64, 64).unwrap(); //aligned to cache line
                alloc::dealloc(self.ptr as *mut u8, layout);
            }
            self.ptr = ptr::null_mut();
        }
    }
}

impl<'a, T: ?Sized + 'a> Drop for WriteGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        self.lock.write_unlock();
    }
}

impl<'a, T: ?Sized + 'a> Drop for ReadGuard<'a, T> {
    #[inline]
    fn drop(&mut self) {
        if self.reader_id < LOCK_COUNT as usize {
            self.lock.fast_local_read_unlock(self.reader_id);
        }
        else {
            self.lock.read_unlock();
        }
    }
}

impl<'a, T: ?Sized> Deref for ReadGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T: ?Sized> Deref for WriteGuard<'a, T> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T: ?Sized> DerefMut for WriteGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

struct RawLocalLock {
    state: AtomicUsize,
    spin_lock: AtomicUsize,
    parked_thread: Option<Thread>,
}

impl RawLocalLock {
    #[inline]
    fn new<'a>(ptr: *mut u64, reader_id: usize) -> &'a mut RawLocalLock {
        unsafe {
            let p = ptr.add(8 + (reader_id << 3));
            &mut *(p as * const _ as *mut RawLocalLock)
        }
    }

    fn init(&mut self) {
        self.state = AtomicUsize::new(GLOBAL_MASK as usize);
        self.spin_lock = AtomicUsize::new(0);
        unsafe {
            ptr::write(&mut self.parked_thread as *mut _, None);
        }
    }

    fn park_thread(&mut self) -> bool {
        let current_thread = thread::current();
        let mut spinwait = SpinWait::new();
        loop {
            if self.spin_lock.compare_exchange(0, 1, Ordering::Release, Ordering::Relaxed).is_ok() {
                let state = self.load_state();
                let valid = state & READER_MASK != 0;
                if !valid {
                    self.spin_lock.store(0, Ordering::Release);
                    return false;
                }
                self.parked_thread = Some(current_thread);
                self.spin_lock.store(0, Ordering::Release);
                thread::park();
                break;
            }
            spinwait.spin();
        }
        true
    }

    fn unpark_thread(&mut self) {
        let mut spinwait = SpinWait::new();
        loop {
            if self.spin_lock.compare_exchange(0, 1, Ordering::Release, Ordering::Relaxed).is_ok() {
                if self.parked_thread.is_some() {
                    let mut t = None;
                    mem::swap(&mut self.parked_thread, & mut t);
                    t.unwrap().unpark();
                }
                self.spin_lock.store(0, Ordering::Release);
                break;
            }
            spinwait.spin();
        }
    }

    #[inline]
    fn cas_state(&self, expect: u32, new_val: u32) -> bool {
        self.state.compare_exchange(expect as usize, new_val as usize, Ordering::Release, Ordering::Relaxed).is_ok()
    }

    #[inline]
    fn load_state(&self) -> u32 {
        self.state.load(Ordering::Relaxed) as u32
    }

    #[inline]
    fn try_read_lock_local(&self) -> bool {
        self.cas_state(0, 1) || self.try_read_lock_local_slow()
    }

    fn try_read_lock_local_slow(&self) -> bool {
        loop {
            let c = self.load_state();
            if (c & GLOBAL_MASK) != 0 { return false; }
            if self.cas_state(c, c + 1) {
                return true;
            }
        }
    }

    #[inline]
    fn read_lock_deglobalize(&self) {
        if self.cas_state(GLOBAL_MASK, 1) { return; }
        self.read_lock_deglobalize_slow();
    }

    fn read_lock_deglobalize_slow(&self) {
        loop {
            let c = self.load_state();
            if self.cas_state(c, (c & !GLOBAL_MASK) + 1) { return; }
        }
    }

    #[inline]
    fn try_deglobalize(&self) {
        self.cas_state(GLOBAL_MASK, 0);
    }

    fn make_global(&self) -> bool {
        loop {
            let c = self.load_state();
            if self.cas_state(c, c | GLOBAL_MASK) {
                return c & READER_MASK == 0;
            }
        }
    }

    fn wait_for_reader(&mut self) {
        if self.load_state() & READER_MASK != 0 {
            self.slow_wait_for_reader();
        }
    }

    fn slow_wait_for_reader(&mut self) {
        let mut parked = false;
        loop {
            if !parked {
                spin_loop_hint();
            }
            if self.load_state() & READER_MASK == 0 {
                return ;
            }
            parked = self.park_thread();
        }
    }

    #[inline(always)]
    fn read_unlock(&self) -> bool {
        if self.cas_state(1, 0) {
            return false;
        }
        self.try_slow_release_shared()
    }

    fn try_slow_release_shared(&self) -> bool {
        loop {
            let c = self.load_state();
            if self.cas_state(c, c - 1) {
                return c == GLOBAL_MASK | 1;
            }
        }
    }

}

struct RawGlobalLock {
    state: AtomicUsize,
    parked: AtomicBool,
    spin_lock: AtomicBool,
    parked_priv_writer: Option<Thread>,
    local_bias: u32,
}

impl RawGlobalLock {

    #[inline]
    fn from_ptr<'a>(ptr: *mut u64) -> &'a mut RawGlobalLock {
        unsafe {
            &mut *(ptr as * const _ as *mut RawGlobalLock)
        }
    }

    #[inline]
    fn init(&mut self) {
        self.state = AtomicUsize::new(GLOBAL_MASK as usize);
        self.local_bias = 0;
        self.parked = AtomicBool::new(false);
        self.spin_lock = AtomicBool::new(false);
        unsafe { ptr::write(&mut self.parked_priv_writer, None); }
    }

    fn park_priv_writer_thread(&mut self) -> bool {
        let current_thread = thread::current();
        let mut spinwait = SpinWait::new();
        loop {
            if self.spin_lock.compare_exchange(false, true, Ordering::Release, Ordering::Relaxed).is_ok() {
                let state = self.load_state();
                let valid = state & READER_MASK != 0;
                if !valid {
                    self.spin_lock.store(false, Ordering::Release);
                    return false;
                }
                self.parked_priv_writer = Some(current_thread);
                self.spin_lock.store(false, Ordering::Release);
                thread::park();
                break;
            }
            spinwait.spin();
        }
        true
    }

    fn unpark_priv_writer_thread(&mut self) {
        let mut spinwait = SpinWait::new();
        loop {
            if self.spin_lock.compare_exchange(false, true, Ordering::Release, Ordering::Relaxed).is_ok() {
                if self.parked_priv_writer.is_some() {
                    let mut t = None;
                    mem::swap(&mut self.parked_priv_writer, & mut t);
                    t.unwrap().unpark();
                }
                self.spin_lock.store(false, Ordering::Release);
                break;
            }
            spinwait.spin();
        }
    }

    #[inline]
    fn cas_state(&self, expect: u32, new_val: u32) -> bool {
        self.state.compare_exchange(expect as usize, new_val as usize, Ordering::Release, Ordering::Relaxed).is_ok()
    }

    #[inline]
    fn load_state(&self) -> u32 {
        self.state.load(Ordering::Relaxed) as u32
    }

    #[inline]
    fn raise_park_flag(&self) {
        self.parked.store(true, Ordering::Release);
    }

    #[inline]
    fn write_lock(&mut self) -> (bool, bool) {
        let (locked, write_prep, must_relock_global) = self.try_write_lock();
        if !locked {
            return self.slow_write_lock();
        }
        (write_prep, must_relock_global)
    }

    #[inline]
    fn try_fast_read_lock(&mut self) -> bool {
        self.local_bias = self.local_bias.saturating_add(1);
        if self.local_bias < 100 {
            let c = self.load_state();
            if self.cas_state(c & !(WRITER_MASK), (c & !(WRITER_MASK)) + 1) {
                return true;
            }
        }
        false
    }

    #[inline]
    fn read_lock(&mut self, local_lock: &RawLocalLock) -> (bool, bool) {
        let (locked, read_prep, local_locked) = self.try_read_lock(local_lock);
        if !locked {
            return self.slow_read_lock(local_lock);
        }
        (read_prep, local_locked)
    }

    #[inline]
    fn try_read_lock(&mut self, local_lock: &RawLocalLock) -> (bool,bool,bool) {
        let c = self.load_state();
        if (c & (WRITER_MASK | PREPARING_TO_GO_LOCAL)) == 0 {
            if self.cas_state(c, (c & !GLOBAL_MASK) | PREPARING_TO_GO_LOCAL) {
                return (true, true, false);
            }
        }
        if c & GLOBAL_MASK == 0 && local_lock.try_read_lock_local() {
            return (true, false, true);
        }
        return (false, false, false);
    }

    #[inline]
    fn write_unlock(&mut self) {
        self.cas_state(GLOBAL_MASK | WRITER_MASK, GLOBAL_MASK);
        self.unpark_all();
    }

    #[inline]
    fn read_unlock_zero(&mut self) {
        if self.try_read_unlock_zero() {
            self.unpark_priv_writer_thread();
        }
        else {
            self.unpark_all();
        }
    }

    #[inline]
    fn wait_for_reader(&mut self) {
        let c = self.load_state();
        if c & READER_MASK != 0 {
            self.slow_wait_for_reader();
        }
        return;
    }

    fn slow_wait_for_reader(&mut self) {
        let mut parked = false;
        loop {
            if !parked {
                spin_loop_hint();
            }
            let state = self.load_state();
            if state & READER_MASK == 0 {
                return;
            }
            parked = self.park_priv_writer_thread();
        }
    }

    fn read_unlock_prep_local(&mut self) {
        loop {
            let c = self.load_state();
            if self.cas_state(c, c & !PREPARING_TO_GO_LOCAL) {
                self.unpark_all();
                return ;
            }
        }
    }

    #[inline]
    fn try_read_unlock_zero(&mut self) -> bool {
        if !self.cas_state(GLOBAL_MASK | 1, GLOBAL_MASK) {
            return self.read_unlock_slow();
        }
        false
    }

    #[inline]
    fn read_unlock_slow(&self) -> bool {
        loop {
            let c = self.load_state();
            if self.cas_state(c, c - 1) { return c & WRITER_MASK != 0; }
        }
    }

    fn try_single_reader_upgrade(&mut self) -> bool {
        if self.cas_state(GLOBAL_MASK | 1, GLOBAL_MASK | WRITER_MASK ) {
            self.local_bias = 0;
            return true;
        }
        false
    }

    //p is expected to be either GLOBAL_MASK, or GLOBAL_MASK | PREPARING_TO_WRITE_MASK
    #[inline]
    fn try_write_lock(&mut self) -> (bool, bool, bool) { // (global state changed, write_prep, must_wait_for_reader)
        self.local_bias = 0;
        let locked = self.cas_state(GLOBAL_MASK, WRITER_MASK | GLOBAL_MASK);
        if locked { return (true, false, false); }
        self.try_write_lock_slow()
    }

    fn try_write_lock_slow(&mut self) -> (bool, bool, bool) {
        loop {
            let c = self.load_state();
            if c & (PREPARING_TO_GO_LOCAL | WRITER_MASK) == 0 {
                if self.cas_state(c, c | WRITER_MASK | GLOBAL_MASK) {
                    return (true, c & GLOBAL_MASK == 0, c & READER_MASK != 0);
                }
            }
            return (false, false, false);
        }
    }

    fn slow_write_lock(&mut self) -> (bool, bool) {
        let mut parked = false;
        loop {
            if !parked {
                spin_loop_hint();
            }
            parked = false;
            let x = self.try_write_lock();
            if x.0 {
                return (x.1, x.2);
            }
            self.raise_park_flag();

            // Park our thread until we are woken up by an unlock
            unsafe {
                let addr = &(self.state) as *const _ as usize;
                let validate = || {
                    let state = self.load_state();
                    let valid = state != 0 && state != GLOBAL_MASK && state != 1;// && state != GLOBAL_MASK | PREPARING_TO_WRITE_MASK;
                    if valid { self.raise_park_flag(); }
                    return valid;
                };
                let before_sleep = || {};
                let timed_out = |_, _| {};
                match parking_lot_core::park(
                    addr,
                    validate,
                    before_sleep,
                    timed_out,
                    WRITE_PARK_TOKEN,
                    None,
                ) {
                    // We were unparked normally, try acquiring the lock again
                    ParkResult::Unparked(_) => {
                        parked = true;
                    },

                    // The validation function failed, try locking again
                    ParkResult::Invalid => (),

                    // Timeout expired
                    ParkResult::TimedOut => panic!(""),
                }
            }
        }
    }

    fn slow_read_lock(&mut self, local_lock: &RawLocalLock) -> (bool, bool) {
        let mut parked = false;
        loop {
            if !parked {
                spin_loop_hint();
            }
            parked = false;
            let x = self.try_read_lock(local_lock);
            if x.0 {
                return (x.1, x.2);
            }
            self.raise_park_flag();

            // Park our thread until we are woken up by an unlock
            unsafe {
                let addr = &(self.state) as *const _ as usize;
                let validate = || {
                    let state = self.load_state();
                    let valid = (state & (WRITER_MASK | PREPARING_TO_GO_LOCAL)) != 0;
                    if valid {
                        self.raise_park_flag();
                    }
                    return valid;
                };
                let before_sleep = || {};
                let timed_out = |_, _| {};
                match parking_lot_core::park(
                    addr,
                    validate,
                    before_sleep,
                    timed_out,
                    READ_PARK_TOKEN,
                    None,
                ) {
                    // We were unparked normally, try acquiring the lock again
                    ParkResult::Unparked(_) => {
                        parked = true;
                    }

                    // The validation function failed, try locking again
                    ParkResult::Invalid => (),

                    // Timeout expired
                    ParkResult::TimedOut => panic!(""),
                }
            }
        }
    }

    #[inline]
    fn unpark_all(&self) {
        let has_parked = self.parked.load(Ordering::Acquire);
        if has_parked {
            self.parked.store(false, Ordering::Release);
            self.force_unpark_all();
        }
    }

    #[inline]
    fn force_unpark_all(&self) {
        unsafe {
            let addr = &(self.state) as *const _ as usize;
            let mut first_writer = false;
            let mut first = true;
            parking_lot_core::unpark_filter(addr, |token| {
                if first {
                    first = false;
                    if WRITE_PARK_TOKEN == token {
                        first_writer = true;
                    }
                    return FilterOp::Unpark;
                } else {
                    if first_writer {
                        return FilterOp::Stop;
                    }
                    if READ_PARK_TOKEN == token {
                        return FilterOp::Unpark;
                    }
                    return FilterOp::Skip;
                }
            }, |res| {
                if res.have_more_threads {
                    self.parked.store(true, Ordering::Release);
                }
                DEFAULT_UNPARK_TOKEN
            });
        }
    }
}

#[inline]
pub fn thread_id_as_u64() -> u64 {
    THREAD_ID.with(|id_cell| *id_cell.borrow() )
}

static ID_COUNT: AtomicUsize = AtomicUsize::new(0);

#[inline]
pub fn thread_id_as_u64_init() -> u64 {
    ID_COUNT.fetch_add(1, Ordering::Release) as u64
}

#[inline]
fn reader_index() -> usize {
    (thread_id_as_u64() & LOCK_MASK as u64) as usize
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn write_lock_read_lock() {
        let lock = WideRwLock::new(());
        {
            lock.write();
        }
        {
            lock.read();
        }
    }
}

