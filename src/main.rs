use std::sync::atomic::{AtomicU32, AtomicU64};

use rppal::gpio::Level;

fn main() {
    let gpio = rppal::gpio::Gpio::new().unwrap();

    let mut pin_high = gpio.get(10).unwrap().into_input();
    pin_high
        .set_async_interrupt(rppal::gpio::Trigger::Both, on_high)
        .unwrap();

    let (sender, receiver) = crossbeam::channel::bounded(1);

    let mut pin_low = gpio.get(11).unwrap().into_input();
    pin_low
        .set_async_interrupt(rppal::gpio::Trigger::Both, move |level| {
            on_low(level, &sender)
        })
        .unwrap();

    while let Ok((line, fb)) = receiver.recv() {
        if line == 0 {
            println!();
        }
        for b in fb {
            print!("{}", if b { '#' } else { ' ' })
        }
        println!()
    }
}

const VIS_WIDTH: usize = 1180;
const LINE_WIDTH: u64 = 1280;
// todo: check this
const LINE_COUNT: u64 = 300;
const HERTZ: u64 = 50;

// frequency of messages
const BIT_RATE: u64 = HERTZ * LINE_WIDTH * LINE_COUNT;

// static LAST_NEW_PAGE: AtomicU64 = AtomicU64::new(0);
static LAST_NEW_LINE: AtomicU64 = AtomicU64::new(0);
// static LINE: AtomicU32 = AtomicU32::new(0);
static LINE: crossbeam::atomic::AtomicCell<[bool; VIS_WIDTH]> =
    crossbeam::atomic::AtomicCell::new([false; VIS_WIDTH]);
static LAST_HIGH_INDEX: AtomicU32 = AtomicU32::new(0);

#[inline(always)]
fn get_time() -> u64 {
    use tock_registers::interfaces::Readable;
    cortex_a::registers::CNTPCT_EL0.get()
}

#[inline(always)]
fn get_freq() -> u64 {
    use tock_registers::interfaces::Readable;
    cortex_a::registers::CNTFRQ_EL0.get()
}

fn on_high(level: Level) {
    // ticks since startup
    let time = get_time();
    let last_new_line = LAST_NEW_LINE.load(std::sync::atomic::Ordering::Relaxed);
    // ticks since last new line
    // todo: add modulo ticks per line
    let diff = time - last_new_line;
    let freq = get_freq();
    // index in current frame (rounded)
    let index = ((diff * freq + BIT_RATE / 2) / BIT_RATE) as u32;

    match level {
        Level::Low => {
            let last_high_index =
                LAST_HIGH_INDEX.swap(VIS_WIDTH as u32, std::sync::atomic::Ordering::Relaxed);
            if index >= last_high_index {
                LINE.fetch_update(|mut a| {
                    let range = index as usize..(last_high_index as usize).min(VIS_WIDTH);
                    let Some(range) = a.get_mut(range) else {
                        return None;
                    };
                    for elem in range {
                        *elem = true;
                    }
                    Some(a)
                })
                .ok();
            }
        }
        Level::High => LAST_HIGH_INDEX.store(index, std::sync::atomic::Ordering::Relaxed),
    }
}

static LAST_LOW: AtomicU64 = AtomicU64::new(0);
static LINE_NUMBER: AtomicU32 = AtomicU32::new(0);

// 1 row blank ~ 5us
// 1 frame blank ~ 1000us
// picking 100us
const TIME_BETWEEN_LOW_TYPES: u64 = 100;
const US_CONV_FACTOR: u64 = 1000000;

type FbSender = crossbeam::channel::Sender<(u32, [bool; VIS_WIDTH])>;

fn on_low(level: Level, fb: &FbSender) {
    // ticks since startup
    let time = get_time();
    match level {
        Level::Low => LAST_LOW.store(time, std::sync::atomic::Ordering::Relaxed),
        Level::High => {
            LAST_NEW_LINE.store(time, std::sync::atomic::Ordering::Relaxed);
            let last_low = LAST_LOW.load(std::sync::atomic::Ordering::Relaxed);
            let freq = get_freq();
            let line = if last_low + freq * TIME_BETWEEN_LOW_TYPES / US_CONV_FACTOR > time {
                // new line
                LINE_NUMBER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            } else {
                // new screen
                LINE_NUMBER.swap(0, std::sync::atomic::Ordering::Relaxed)
            };
            let data = LINE.swap([false; VIS_WIDTH]);
            if line < LINE_COUNT as u32 {
                fb.send((line, data)).ok();
            }
        }
    }
}
