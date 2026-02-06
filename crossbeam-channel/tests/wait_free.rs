use std::thread;

use crossbeam_channel::{wait_free_bounded, TryRecvError, TrySendError};

#[test]
fn registration_and_round_robin() {
    let (registry, mut receiver) = wait_free_bounded::<i32>(2, 2);

    assert_eq!(registry.total_slots(), 2);
    assert_eq!(registry.remaining_slots(), 2);

    let sender0 = registry.register().expect("missing slot 0");
    let sender1 = registry.register().expect("missing slot 1");
    assert!(registry.register().is_none());
    assert_eq!(registry.remaining_slots(), 0);

    sender0.try_send(1).unwrap();
    sender0.try_send(2).unwrap();
    sender1.try_send(10).unwrap();

    assert_eq!(receiver.try_recv(), Ok(1));
    assert_eq!(receiver.try_recv(), Ok(10));
    assert_eq!(receiver.try_recv(), Ok(2));
}

#[test]
fn full_and_disconnected_states() {
    let (registry, mut receiver) = wait_free_bounded::<i32>(1, 1);
    let sender = registry.register().expect("missing sender slot");

    assert_eq!(sender.try_send(11), Ok(()));
    assert_eq!(sender.try_send(22), Err(TrySendError::Full(22)));
    assert_eq!(receiver.try_recv(), Ok(11));
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));

    drop(sender);
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));

    registry.close();
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}

#[test]
fn receiver_drop_disconnects_sender() {
    let (registry, receiver) = wait_free_bounded::<i32>(1, 1);
    let sender = registry.register().expect("missing sender slot");
    drop(receiver);

    assert_eq!(sender.try_send(7), Err(TrySendError::Disconnected(7)));
}

#[test]
fn concurrent_senders_deliver_all_messages() {
    #[cfg(miri)]
    const COUNT: i32 = 100;
    #[cfg(not(miri))]
    const COUNT: i32 = 5_000;

    let (registry, mut receiver) = wait_free_bounded::<i32>(2, 64);
    let sender0 = registry.register().expect("missing sender slot");
    let sender1 = registry.register().expect("missing sender slot");
    drop(registry);

    let handle0 = thread::spawn(move || {
        for i in 0..COUNT {
            let mut msg = i;
            loop {
                match sender0.try_send(msg) {
                    Ok(()) => break,
                    Err(TrySendError::Full(v)) => {
                        msg = v;
                        thread::yield_now();
                    }
                    Err(TrySendError::Disconnected(_)) => panic!("receiver disconnected early"),
                }
            }
        }
    });

    let handle1 = thread::spawn(move || {
        for i in 0..COUNT {
            let mut msg = i + 100_000;
            loop {
                match sender1.try_send(msg) {
                    Ok(()) => break,
                    Err(TrySendError::Full(v)) => {
                        msg = v;
                        thread::yield_now();
                    }
                    Err(TrySendError::Disconnected(_)) => panic!("receiver disconnected early"),
                }
            }
        }
    });

    let mut received = 0;
    let mut sum = 0_i64;
    while received < COUNT * 2 {
        match receiver.try_recv() {
            Ok(msg) => {
                received += 1;
                sum += msg as i64;
            }
            Err(TryRecvError::Empty) => thread::yield_now(),
            Err(TryRecvError::Disconnected) => panic!("channel disconnected early"),
        }
    }

    handle0.join().unwrap();
    handle1.join().unwrap();

    let sum0 = (COUNT as i64 - 1) * COUNT as i64 / 2;
    let sum1 = (COUNT as i64 - 1) * COUNT as i64 / 2 + (COUNT as i64 * 100_000);
    assert_eq!(sum, sum0 + sum1);
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}
