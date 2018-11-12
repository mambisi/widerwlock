// Copyright 2018 Mohammad Rezaei.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.
//

#![feature(test)]
extern crate test;
extern crate widerwlock;
extern crate parking_lot;
extern crate rand;
extern crate xoshiro;

use test::Bencher;
use test::stats::Summary;
use test::black_box;

use widerwlock::*;
use parking_lot::*;
use std::sync::mpsc::*;
use std::sync::Mutex;
use std::ops::Deref;
use parking_lot::RwLock;
use std::thread;
use std::cell::RefCell;
use std::sync::Arc;
use xoshiro::Xoshiro512StarStar;
use rand::*;


thread_local!(static FOO: RefCell<u64> = RefCell::new(widerwlock::thread_id_as_u64()));

#[bench]
fn bench_uncontended_read_grwl(b: &mut Bencher) {
    let lock = WideRwLock::new(());
    b.iter(|| {
        for _i in 0..1_000_000 {
            lock.read();
        }
    })
}

#[bench]
fn bench_uncontended_write_grwl(b: &mut Bencher) {
    let lock = WideRwLock::new(());
    b.iter(|| {
        for _i in 0..1_000_000 {
            lock.write();
        }
    })
}

#[bench]
fn bench_uncontended_read_std(b: &mut Bencher) {
    let lock = std::sync::RwLock::new(());
    b.iter(|| {
        for _i in 0..1_000_000 {
            lock.read();
        }
    })
}

#[bench]
fn bench_uncontended_write_std(b: &mut Bencher) {
    let lock = std::sync::RwLock::new(());
    b.iter(|| {
        for _i in 0..1_000_000 {
            lock.write();
        }
    })
}

#[bench]
fn bench_uncontended_mutex(b: &mut Bencher) {
    let mutex = Mutex::new(());
    b.iter(|| {
        for _i in 0..1_000_000 {
            mutex.lock();
        }
    })
}

#[bench]
fn bench_uncontended_read_pk(b: &mut Bencher) {
    let lock = RwLock::new(());
    b.iter(|| {
        for _i in 0..1_000_000 {
            lock.read();
        }
    })
}

#[bench]
fn bench_uncontended_write_pk(b: &mut Bencher) {
    let lock = RwLock::new(());
    b.iter(|| {
        for _i in 0..1_000_000 {
            lock.write();
        }
    })
}

#[bench]
fn bench_2_reader_grwl(b: &mut Bencher) {
    run_multi_reader_grwl(b, 2);
}

#[bench]
fn bench_4_reader_grwl(b: &mut Bencher) {
    run_multi_reader_grwl(b, 4);
}

#[bench]
fn bench_8_reader_grwl(b: &mut Bencher) {
    run_multi_reader_grwl(b, 8);
}

#[bench]
fn bench_2_reader_pk(b: &mut Bencher) {
    run_multi_reader_pk(b, 2);
}

#[bench]
fn bench_4_reader_pk(b: &mut Bencher) {
    run_multi_reader_pk(b, 4);
}

#[bench]
fn bench_8_reader_pk(b: &mut Bencher) {
    run_multi_reader_pk(b, 8);
}

#[bench]
fn bench_2_reader_std(b: &mut Bencher) {
    run_multi_reader_std(b, 2);
}

#[bench]
fn bench_4_reader_std(b: &mut Bencher) {
    run_multi_reader_std(b, 4);
}

#[bench]
fn bench_8_reader_std(b: &mut Bencher) {
    run_multi_reader_std(b, 8);
}

fn run_multi_reader_grwl(b: &mut Bencher, threads: u32) {
    b.iter(|| {
        let r = Arc::new(WideRwLock::new(()));

        let (tx, rx) = channel::<()>();
        for _ in 0..threads {
            let tx = tx.clone();
            let r = r.clone();
            thread::spawn(move || {
                for _ in 0..1_000_000 {
                    drop(r.read());
                }
                drop(tx);
            });
        }
        drop(tx);
        let _ = rx.recv();
    });
}

fn run_multi_reader_pk(b: &mut Bencher, threads: u32) {
    b.iter(|| {
        let r = Arc::new(RwLock::new(()));

        let (tx, rx) = channel::<()>();
        for _ in 0..threads {
            let tx = tx.clone();
            let r = r.clone();
            thread::spawn(move || {
                for _ in 0..1_000_000 {
                    drop(r.read());
                }
                drop(tx);
            });
        }
        drop(tx);
        let _ = rx.recv();
    });
}

fn run_multi_reader_std(b: &mut Bencher, threads: u32) {
    b.iter(|| {
        let r = Arc::new(std::sync::RwLock::new(()));

        let (tx, rx) = channel::<()>();
        for _ in 0..threads {
            let tx = tx.clone();
            let r = r.clone();
            thread::spawn(move || {
                for _ in 0..1_000_000 {
                    drop(r.read());
                }
                drop(tx);
            });
        }
        drop(tx);
        let _ = rx.recv();
    });
}

//#[bench]
fn bench_thread_id(b: &mut Bencher) {
    b.iter(|| {
        for _i in 0..1_000_000 {
            widerwlock::thread_id_as_u64();
        }
    })
}

//#[bench]
fn bench_thread_current(b: &mut Bencher) {
    b.iter(|| {
        for _i in 0..1_000_000 {
            thread::current();
        }
    })
}

//#[bench]
fn bench_thread_local_borrow(b: &mut Bencher) {
    b.iter(|| {
        FOO.with(|f| black_box(*f.borrow()));
    })
}

#[bench]
fn bench_mixed_tenth_work10_1_grwl(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_grwl(1, 0.1);
    })
}

#[bench]
fn bench_mixed_hundreth_work10_1_grwl(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_grwl(1, 0.01);
    })
}

#[bench]
fn bench_mixed_tenth_work10_4_grwl(b: &mut Bencher) {
//    read_write_no_work_grwl(4, 0.1);

    b.iter(|| {
        read_write_work10_grwl(4, 0.1);
    })
}

#[bench]
fn bench_mixed_hundreth_work10_4_grwl(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_grwl(4, 0.01);
    })
}

#[bench]
fn bench_mixed_tenth_work100_4_grwl(b: &mut Bencher) {
    b.iter(|| {
        read_write_work100_grwl(4, 0.1);
    })
}

#[bench]
fn bench_mixed_tenth_work10_1_pk(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_pk(1, 0.1);
    })
}

#[bench]
fn bench_mixed_hundreth_work10_1_pk(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_pk(1, 0.01);
    })
}

#[bench]
fn bench_mixed_tenth_work10_4_pk(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_pk(4, 0.1);
    })
}

#[bench]
fn bench_mixed_tenth_work100_4_pk(b: &mut Bencher) {
    b.iter(|| {
        read_write_work100_pk(4, 0.1);
    })
}

#[bench]
fn bench_mixed_hundreth_work10_4_pk(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_pk(4, 0.01);
    })
}

#[bench]
fn bench_mixed_tenth_work10_1_std(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_std(1, 0.1);
    })
}

#[bench]
fn bench_mixed_hundreth_work10_1_std(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_std(1, 0.01);
    })
}

#[bench]
fn bench_mixed_tenth_work10_4_std(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_std(4, 0.1);
    })
}

#[bench]
fn bench_mixed_tenth_work100_4_std(b: &mut Bencher) {
    b.iter(|| {
        read_write_work100_std(4, 0.1);
    })
}

#[bench]
fn bench_mixed_hundreth_work10_4_std(b: &mut Bencher) {
    b.iter(|| {
        read_write_work10_std(4, 0.01);
    })
}

fn read_write_work10_grwl(threads: u64, writer_fraction: f64) {
    const M: u32 = 1_000_000;

    let r = Arc::new(WideRwLock::new(0u128));

    let (tx, rx) = channel::<()>();
    for t in 0..threads {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let mut sum: f64 = 0.0;
            let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF ^ t);
            for _ in 0..M {
                if rng.gen::<f64>() < writer_fraction {
                    r.write();
                    sum += work10(&mut rng);
                } else {
                    r.read();
                    sum += work10(&mut rng);
                }
            }
            assert!(sum != 0.0);
            drop(tx);
        });
    }
    drop(tx);
    let _ = rx.recv();
}

fn read_write_work100_grwl(threads: u64, writer_fraction: f64) {
    const M: u32 = 1_000_00;

    let r = Arc::new(WideRwLock::new(()));

    let (tx, rx) = channel::<()>();
    for t in 0..threads {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let mut sum: f64 = 0.0;
            let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF ^ t);
            for _ in 0..M {
                if rng.gen::<f64>() < writer_fraction {
                    r.write();
                    sum += work100(&mut rng);
                } else {
                    r.read();
                    sum += work100(&mut rng);
                }
            }
            assert!(sum > 0.0);
            drop(tx);
        });
    }
    drop(tx);
    let _ = rx.recv();
}

fn read_write_work10_pk(threads: u64, writer_fraction: f64) {
    const M: u32 = 1_000_000;

    let r = Arc::new(RwLock::new(0u128));

    let (tx, rx) = channel::<()>();
    for t in 0..threads {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let mut sum: f64 = 0.0;
            let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF ^ t);
            for _ in 0..M {
                if rng.gen::<f64>() < writer_fraction {
                    r.write();
                    sum += work10(&mut rng);
                } else {
                    r.read();
                    sum += work10(&mut rng);
                }
            }
            assert!(sum != 0.0);
            drop(tx);
        });
    }
    drop(tx);
    let _ = rx.recv();
}

fn read_write_work100_pk(threads: u64, writer_fraction: f64) {
    const M: u32 = 1_000_00;

    let r = Arc::new(RwLock::new(()));

    let (tx, rx) = channel::<()>();
    for t in 0..threads {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let mut sum: f64 = 0.0;
            let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF ^ t);
            for _ in 0..M {
                if rng.gen::<f64>() < writer_fraction {
                    r.write();
                    sum += work100(&mut rng);
                } else {
                    r.read();
                    sum += work100(&mut rng);
                }
            }
            assert!(sum > 0.0);
            drop(tx);
        });
    }
    drop(tx);
    let _ = rx.recv();
}

fn read_write_work10_std(threads: u64, writer_fraction: f64) {
    const M: u32 = 1_000_000;

    let r = Arc::new(std::sync::RwLock::new(0u128));

    let (tx, rx) = channel::<()>();
    for t in 0..threads {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let mut sum: f64 = 0.0;
            let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF ^ t);
            for _ in 0..M {
                if rng.gen::<f64>() < writer_fraction {
                    r.write();
                    sum += work10(&mut rng);
                } else {
                    r.read();
                    sum += work10(&mut rng);
                }
            }
            assert!(sum != 0.0);
            drop(tx);
        });
    }
    drop(tx);
    let _ = rx.recv();
}

fn read_write_work100_std(threads: u64, writer_fraction: f64) {
    const M: u32 = 1_000_00;

    let r = Arc::new(std::sync::RwLock::new(()));

    let (tx, rx) = channel::<()>();
    for t in 0..threads {
        let tx = tx.clone();
        let r = r.clone();
        thread::spawn(move || {
            let mut sum: f64 = 0.0;
            let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF ^ t);
            for _ in 0..M {
                if rng.gen::<f64>() < writer_fraction {
                    r.write();
                    sum += work100(&mut rng);
                } else {
                    r.read();
                    sum += work100(&mut rng);
                }
            }
            assert!(sum != 0.0);
            drop(tx);
        });
    }
    drop(tx);
    let _ = rx.recv();
}

#[bench]
fn bench_work100(b: &mut Bencher) {
    let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF);
    b.iter(|| {
        work100(&mut rng);
    })
}

#[bench]
fn bench_work10(b: &mut Bencher) {
    let mut rng = Xoshiro512StarStar::from_seed_u64(0xCAFE_BABE_DEAD_BEEF);
    b.iter(|| {
        work10(&mut rng);
    })
}

#[cold]
fn work100(rng: &mut Xoshiro512StarStar) -> f64 {
    let max: i32 = (2.0 + rng.gen::<f64>() * 1.5) as i32;
    let mut sum: f64 = 0.0;
    for _i in 0..max {
        sum += rng.gen::<f64>().cbrt();
    }
    sum
}

#[cold]
fn work10(rng: &mut Xoshiro512StarStar) -> f64 {
    let max: i32 = (2.0 + rng.gen::<f64>() * 1.5) as i32;
    let mut sum: f64 = 0.0;
    for _i in 0..max {
        sum += rng.gen::<f64>().sqrt();
    }
    sum
}
