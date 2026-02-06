//! Experimental wait-free channel flavor with fixed sender slots.
//!
//! This API intentionally supports only non-blocking operations.

use std::boxed::Box;
use std::cell::UnsafeCell;
use std::fmt;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::vec::Vec;

use crossbeam_utils::CachePadded;

use crate::err::{TryRecvError, TrySendError};

/// Creates a fixed-topology wait-free channel.
///
/// The channel is configured with `sender_slots` independent SPSC queues. A sender must first be
/// registered into a slot via [`WaitFreeSenderRegistry::register`]. The receiver polls all slots in
/// a round-robin pattern.
///
/// This channel supports only non-blocking operations: [`WaitFreeSender::try_send`] and
/// [`WaitFreeReceiver::try_recv`].
///
/// # Panics
///
/// Panics if `sender_slots == 0` or `capacity_per_sender == 0`.
pub fn wait_free_bounded<T>(
    sender_slots: usize,
    capacity_per_sender: usize,
) -> (WaitFreeSenderRegistry<T>, WaitFreeReceiver<T>) {
    assert!(sender_slots > 0, "sender_slots must be positive");
    assert!(
        capacity_per_sender > 0,
        "capacity_per_sender must be positive"
    );

    let mut slots = Vec::with_capacity(sender_slots);
    for _ in 0..sender_slots {
        slots.push(SenderSlot {
            queue: SpscQueue::with_capacity(capacity_per_sender),
            active: AtomicBool::new(false),
        });
    }

    let inner = Arc::new(Inner {
        slots: slots.into_boxed_slice(),
        next_slot: AtomicUsize::new(0),
        active_senders: AtomicUsize::new(0),
        registration_closed: AtomicBool::new(false),
        receiver_alive: AtomicBool::new(true),
    });

    (
        WaitFreeSenderRegistry {
            inner: Arc::clone(&inner),
        },
        WaitFreeReceiver {
            inner,
            next_slot: 0,
        },
    )
}

/// Registration handle used to claim sender slots.
pub struct WaitFreeSenderRegistry<T> {
    inner: Arc<Inner<T>>,
}

impl<T> fmt::Debug for WaitFreeSenderRegistry<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitFreeSenderRegistry")
            .field("total_slots", &self.total_slots())
            .field("remaining_slots", &self.remaining_slots())
            .finish()
    }
}

impl<T> WaitFreeSenderRegistry<T> {
    /// Tries to register a sender into an available slot.
    ///
    /// Returns `None` if registration is closed or all slots are already claimed.
    pub fn register(&self) -> Option<WaitFreeSender<T>> {
        loop {
            if self.inner.registration_closed.load(Ordering::Acquire) {
                return None;
            }

            let current = self.inner.next_slot.load(Ordering::Acquire);
            if current >= self.inner.slots.len() {
                return None;
            }

            match self.inner.next_slot.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(slot) => {
                    let was_active = self.inner.slots[slot].active.swap(true, Ordering::AcqRel);
                    debug_assert!(!was_active, "sender slot was claimed more than once");
                    self.inner.active_senders.fetch_add(1, Ordering::AcqRel);
                    return Some(WaitFreeSender {
                        inner: Arc::clone(&self.inner),
                        slot,
                    });
                }
                Err(_) => continue,
            }
        }
    }

    /// Prevents future sender registrations.
    pub fn close(&self) {
        self.inner
            .registration_closed
            .store(true, Ordering::Release);
    }

    /// Returns the total number of sender slots.
    pub fn total_slots(&self) -> usize {
        self.inner.slots.len()
    }

    /// Returns how many sender slots are still unclaimed.
    pub fn remaining_slots(&self) -> usize {
        let claimed = self.inner.next_slot.load(Ordering::Acquire);
        self.inner.slots.len().saturating_sub(claimed)
    }
}

impl<T> Drop for WaitFreeSenderRegistry<T> {
    fn drop(&mut self) {
        self.inner
            .registration_closed
            .store(true, Ordering::Release);
    }
}

/// Sender registered in exactly one slot.
pub struct WaitFreeSender<T> {
    inner: Arc<Inner<T>>,
    slot: usize,
}

impl<T> fmt::Debug for WaitFreeSender<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitFreeSender")
            .field("slot", &self.slot)
            .finish()
    }
}

impl<T> WaitFreeSender<T> {
    /// Tries to send a message without blocking.
    pub fn try_send(&self, msg: T) -> Result<(), TrySendError<T>> {
        if !self.inner.receiver_alive.load(Ordering::Acquire) {
            return Err(TrySendError::Disconnected(msg));
        }

        match self.inner.slots[self.slot].queue.try_push(msg) {
            Ok(()) => Ok(()),
            Err(msg) => {
                if !self.inner.receiver_alive.load(Ordering::Acquire) {
                    Err(TrySendError::Disconnected(msg))
                } else {
                    Err(TrySendError::Full(msg))
                }
            }
        }
    }
}

impl<T> Drop for WaitFreeSender<T> {
    fn drop(&mut self) {
        let slot = &self.inner.slots[self.slot];
        if slot.active.swap(false, Ordering::AcqRel) {
            self.inner.active_senders.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

/// Single receiver that polls sender slots in a round-robin pattern.
pub struct WaitFreeReceiver<T> {
    inner: Arc<Inner<T>>,
    next_slot: usize,
}

impl<T> fmt::Debug for WaitFreeReceiver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WaitFreeReceiver")
            .field("next_slot", &self.next_slot)
            .finish()
    }
}

impl<T> WaitFreeReceiver<T> {
    /// Tries to receive a message without blocking.
    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        let len = self.inner.slots.len();

        for step in 0..len {
            let slot = (self.next_slot + step) % len;
            if let Some(msg) = self.inner.slots[slot].queue.try_pop() {
                self.next_slot = (slot + 1) % len;
                return Ok(msg);
            }
        }

        if self.is_disconnected() {
            Err(TryRecvError::Disconnected)
        } else {
            Err(TryRecvError::Empty)
        }
    }

    /// Returns `true` if no future messages can be sent.
    pub fn is_disconnected(&self) -> bool {
        self.inner.registration_closed.load(Ordering::Acquire)
            && self.inner.active_senders.load(Ordering::Acquire) == 0
    }
}

impl<T> Drop for WaitFreeReceiver<T> {
    fn drop(&mut self) {
        self.inner.receiver_alive.store(false, Ordering::Release);
    }
}

struct Inner<T> {
    slots: Box<[SenderSlot<T>]>,
    next_slot: AtomicUsize,
    active_senders: AtomicUsize,
    registration_closed: AtomicBool,
    receiver_alive: AtomicBool,
}

struct SenderSlot<T> {
    queue: SpscQueue<T>,
    active: AtomicBool,
}

struct SpscQueue<T> {
    buffer: Box<[UnsafeCell<MaybeUninit<T>>]>,
    cap: usize,
    head: CachePadded<AtomicUsize>,
    tail: CachePadded<AtomicUsize>,
}

impl<T> SpscQueue<T> {
    fn with_capacity(cap: usize) -> Self {
        let mut buffer = Vec::with_capacity(cap);
        for _ in 0..cap {
            buffer.push(UnsafeCell::new(MaybeUninit::uninit()));
        }

        Self {
            buffer: buffer.into_boxed_slice(),
            cap,
            head: CachePadded::new(AtomicUsize::new(0)),
            tail: CachePadded::new(AtomicUsize::new(0)),
        }
    }

    fn try_push(&self, msg: T) -> Result<(), T> {
        let head = self.head.load(Ordering::Acquire);
        let tail = self.tail.load(Ordering::Relaxed);

        if tail.wrapping_sub(head) >= self.cap {
            return Err(msg);
        }

        let index = tail % self.cap;
        let slot = unsafe { self.buffer.get_unchecked(index) };
        unsafe { slot.get().write(MaybeUninit::new(msg)) };
        self.tail.store(tail.wrapping_add(1), Ordering::Release);
        Ok(())
    }

    fn try_pop(&self) -> Option<T> {
        let tail = self.tail.load(Ordering::Acquire);
        let head = self.head.load(Ordering::Relaxed);

        if head == tail {
            return None;
        }

        let index = head % self.cap;
        let slot = unsafe { self.buffer.get_unchecked(index) };
        let msg = unsafe { slot.get().read().assume_init() };
        self.head.store(head.wrapping_add(1), Ordering::Release);
        Some(msg)
    }
}

impl<T> Drop for SpscQueue<T> {
    fn drop(&mut self) {
        let mut head = *self.head.get_mut();
        let tail = *self.tail.get_mut();

        while head != tail {
            let index = head % self.cap;
            let slot = unsafe { self.buffer.get_unchecked_mut(index) };
            unsafe { slot.get_mut().assume_init_drop() };
            head = head.wrapping_add(1);
        }
    }
}

unsafe impl<T: Send> Send for SpscQueue<T> {}
unsafe impl<T: Send> Sync for SpscQueue<T> {}
