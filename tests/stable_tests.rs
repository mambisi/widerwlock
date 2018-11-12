// Copyright 2018 Mohammad Rezaei.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//

extern crate widerwlock;
extern crate rand;
extern crate xoshiro;

use widerwlock::*;
use std::sync::Arc;
use std::sync::mpsc::*;
use std::thread;
use rand::*;
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;
use std::borrow::*;
use std::sync::atomic::Ordering;
use std::time::{SystemTime, UNIX_EPOCH};
use xoshiro::Xoshiro512StarStar;
use std::collections::HashMap;
use std::hash::Hash;

#[test]
fn test_upgrade() {
    let lock = WideRwLock::new(());
    {
        let guard = lock.read();
        assert!(guard.upgrade_to_write_lock().atomic());
    }
}

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

#[test]
fn test_upgrade2() {
    let lock = WideRwLock::new(());
    {
        lock.read();
        lock.read();// this is not a test of reentrancy. This only works because the lock is uncontended
        lock.read();
    }
    // lock is now in local mode
    {
        lock.write();
    }
    //lock is back in global mode
    let guard = lock.read();
    assert!(guard.upgrade_to_write_lock().atomic());
}

#[test]
fn ten_threads() {
    const N: u32 = 10;
    const M: u32 = 100_000;

    let r = Arc::new(WideRwLock::new(()));

    let (tx, rx) = channel::<()>();
    for _ in 0..N {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let mut rng = rand::thread_rng();
            for _ in 0..M {
                if rng.gen_bool(1.0 / N as f64) {
                    drop(r.write());
                } else {
                    drop(r.read());
                }
            }
            drop(tx);
        });
    }
    drop(tx);
    let _ = rx.recv();
}

#[test]
fn test_multi_threads()
{
    const THREADS: u64 = 50;

    let r = Arc::new(WideRwLock::new(()));

    let (tx, rx) = channel::<bool>();

    for t in 0..THREADS {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let seed = now() ^ t;
            let mut runner = TestRunner::new(r, seed);
            let sum = runner.run();
            assert!(!sum.is_nan());
            tx.send(runner.is_bad());
            drop(tx);
        });
    }
    drop(tx);
    for _x in 0..THREADS {
        assert!(!rx.recv().unwrap());
    }
}

struct LockTester {
    mutex: Mutex<LockTesterData>,
    writer: AtomicBool,
}

unsafe impl Send for LockTester {}
unsafe impl Sync for LockTester {}


struct LockTesterData {
    readers: i32,
    max_readers: i32
}

impl LockTesterData {
    fn new() -> LockTesterData {
        LockTesterData {
            readers: 0,
            max_readers: 0
        }
    }
}

impl LockTester {
    fn new() -> LockTester {
        LockTester { mutex: Mutex::new(LockTesterData::new()),
            writer: AtomicBool::new(false),
        }
    }

    fn get_max_readers(&self) -> i32 {
        let guard = self.mutex.lock().unwrap();
        guard.borrow().max_readers
    }

    fn get_readers(&self) -> i32 {
        let guard = self.mutex.lock().unwrap();
        guard.borrow().readers
    }

    fn is_writer(&self) -> bool {
        self.writer.load(Ordering::Acquire)
    }

    fn start_reader(&self) -> Result<(), &str> {
        if self.is_writer() {
            return Err("can't read while writing!");
        }
        let mut guard = self.mutex.lock().unwrap();
        let data = guard.borrow_mut();
        data.readers += 1;
        if data.readers > data.max_readers {
            data.max_readers = data.readers;
        }
        Ok(())
    }

    fn end_reader(&self) -> Result<(), &str> {
        let mut guard = self.mutex.lock().unwrap();
        let data = guard.borrow_mut();
        data.readers -= 1;
        if data.readers < 0 {
            return Err("readers are out of wack!");
        }
        Ok(())
    }

    fn start_writer(&self) -> Result<(), &str> {
        if self.is_writer() {
            return Err("too many writers!");
        }
        {
            let guard = self.mutex.lock().unwrap();
            let data = guard.borrow();
            if data.readers > 0 {
                return Err("readers are out of wack!");
            }
        }
        self.writer.store(true, Ordering::Release);
        Ok(())
    }

    fn end_writer(&self) -> Result<(), &str> {
        if !self.is_writer() {
            return Err("not writing!");
        }
        {
            let guard = self.mutex.lock().unwrap();
            let data = guard.borrow();
            if data.readers > 0 {
                return Err("readers are out of wack!");
            }
        }
        self.writer.store(false, Ordering::Release);
        Ok(())
    }
}

struct TestRunner {
    bad: AtomicBool,
    rng: Xoshiro512StarStar,
    lock: Arc<WideRwLock<()>>,
    tester: LockTester,
    last_type: f64,
    current_type: f64
}

impl TestRunner {

    fn new(lock: Arc<WideRwLock<()>>, seed: u64) -> TestRunner {
        TestRunner {
            bad: AtomicBool::new(false),
            rng: Xoshiro512StarStar::from_seed_u64(seed),
            lock,
            tester: LockTester::new(),
            last_type: 0.0,
            current_type: 0.0
        }
    }

    fn is_bad(&self) -> bool {
        self.bad.load(Ordering::Acquire)
    }

    fn next_type(&mut self) -> f64 {
        self.rng.gen()
    }

    #[cold]
    fn work(rng: &mut Xoshiro512StarStar) -> f64 {
        let max: i32 = (500.0 + rng.gen::<f64>() * 500.0) as i32;
        let mut sum: f64 = 0.0;
        for _i in 0..max {
            sum += rng.gen::<f64>().cbrt();
        }
        sum
    }

    fn run(&mut self) -> f64 {
        let mut sum: f64 = 0.0;
        for _i in 0..1000 {
            let typ = self.next_type();
            self.last_type = self.current_type;
            self.current_type = typ;
            if typ < 0.1 {
                self.lock.write();
                self.tester.start_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
            }
            else if typ < 0.2 {
                let guard = self.lock.read();
                self.tester.start_reader().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_reader().
                    or_else(|err| self.handle_error(err)).unwrap();
                guard.upgrade_to_write_lock();
                self.tester.start_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
            }
            else if typ < 0.3 {
                self.lock.write();
                self.tester.start_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                self.tester.start_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
            }
            else if typ < 0.4 {
                let guard = self.lock.read();
                self.tester.start_reader().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_reader().
                    or_else(|err| self.handle_error(err)).unwrap();
                guard.upgrade_to_write_lock();
                self.tester.start_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                self.tester.start_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_writer().
                    or_else(|err| self.handle_error(err)).unwrap();
            }
            else {
                self.lock.read();
                self.tester.start_reader().
                    or_else(|err| self.handle_error(err)).unwrap();
                sum += <TestRunner>::work(&mut self.rng);
                self.tester.end_reader().
                    or_else(|err| self.handle_error(err)).unwrap();
            }
        }

        sum
    }

    fn handle_error(&self, err: &str) -> Result<(), ()> {
        println!("{}", err);
        self.bad.store(true, Ordering::Release);
        Ok(())
    }
}

fn now() -> u64 {
    let now = SystemTime::now();
    let dur = now.duration_since(UNIX_EPOCH).unwrap();
    dur.as_secs() * 1000 + dur.subsec_nanos() as u64 / 1_000_000
}

struct RwMap<K,V> {
    map: WideRwLock<HashMap<K,V>>,
}

impl<K: Eq + Hash,V> RwMap<K, V> {
    // the atomic return allows two important things:
    // 1) ensures that the factory function runs only if there is no value in the map
    // 2) the (potentially) expensive double read can be avoided when atomic
    pub fn insert_if_absent<F>(&self, key: K, v_factory: F)
        where F: FnOnce(&K) -> V {
        let guard: ReadGuard<_> = self.map.read();
        let found;
        {
            found = guard.get(&key).is_some();
        }
        if !found {
            let result: UpgradeResult<_> = guard.upgrade_to_write_lock();
            let atomic = result.atomic();
            let mut write_guard: WriteGuard<_> = result.into_guard();
            if !atomic {
                // we have to check again, because another writer may have come in
                if write_guard.get(&key).is_some() { return; }
            }
            let v = v_factory(&key);
            write_guard.insert(key, v);
        }
    }
}