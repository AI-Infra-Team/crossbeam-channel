//! Experimental fixed-topology wait-free channel usage.

use std::thread;

use crossbeam_channel::{wait_free_bounded, TryRecvError, TrySendError};

fn main() {
    let (registry, mut receiver) = wait_free_bounded::<u64>(2, 32);

    let sender0 = registry.register().expect("missing sender slot");
    let sender1 = registry.register().expect("missing sender slot");
    registry.close();

    let handle0 = thread::spawn(move || {
        for value in 0..100 {
            let mut pending = value;
            loop {
                match sender0.try_send(pending) {
                    Ok(()) => break,
                    Err(TrySendError::Full(v)) => {
                        pending = v;
                        thread::yield_now();
                    }
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
        }
    });

    let handle1 = thread::spawn(move || {
        for value in 100..200 {
            let mut pending = value;
            loop {
                match sender1.try_send(pending) {
                    Ok(()) => break,
                    Err(TrySendError::Full(v)) => {
                        pending = v;
                        thread::yield_now();
                    }
                    Err(TrySendError::Disconnected(_)) => return,
                }
            }
        }
    });

    let mut received = 0_u64;
    while received < 200 {
        match receiver.try_recv() {
            Ok(msg) => {
                println!("{msg}");
                received += 1;
            }
            Err(TryRecvError::Empty) => thread::yield_now(),
            Err(TryRecvError::Disconnected) => break,
        }
    }

    handle0.join().unwrap();
    handle1.join().unwrap();
}
