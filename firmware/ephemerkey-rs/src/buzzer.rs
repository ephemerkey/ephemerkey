//! Buzzer on PB4 = TIM3_CH1, PWM through the Q2 low-side driver.
//!
//! Scaffold: a short boot chirp proves the timer/AF routing, then parks.

use defmt::info;
use embassy_stm32::gpio::OutputType;
use embassy_stm32::peripherals::{PB4, TIM3};
use embassy_stm32::time::Hertz;
use embassy_stm32::timer::simple_pwm::{PwmPin, SimplePwm};
use embassy_stm32::Peri;
use embassy_time::Timer;

#[embassy_executor::task]
pub async fn task(tim: Peri<'static, TIM3>, pin: Peri<'static, PB4>) {
    let ch1 = PwmPin::new(pin, OutputType::PushPull);
    let mut pwm = SimplePwm::new(
        tim,
        Some(ch1),
        None,
        None,
        None,
        Hertz::khz(4), // near the CMT-8504 resonance
        Default::default(),
    );

    let mut ch = pwm.ch1();
    ch.set_duty_cycle_percent(50);
    ch.enable();
    Timer::after_millis(60).await;
    ch.disable();
    info!("buzzer: boot chirp done");
    // Parked; later exposed as a command/alert channel via embassy-sync.
}
