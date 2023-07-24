#![no_std]
#![no_main]

extern crate alloc;

use core::time::Duration;
use embedded_graphics::{primitives::{PrimitiveStyleBuilder, Circle, StyledDrawable}, pixelcolor::BinaryColor, prelude::Point};
use kernel::{services::emb_display::{EmbDisplayClient, MonoChunk, FrameLocSize}, Kernel};
use mnemos_d1_core::{
    dmac::Dmac,
    drivers::{spim::kernel_spim1, twi, uart::kernel_uart, sharp_display::SharpDisplay},
    plic::Plic,
    timer::Timers,
    Ram, D1,
};

const HEAP_SIZE: usize = 384 * 1024 * 1024;

#[link_section = ".aheap.AHEAP"]
#[used]
static AHEAP_BUF: Ram<HEAP_SIZE> = Ram::new();

#[allow(non_snake_case)]
#[riscv_rt::entry]
fn main() -> ! {
    unsafe {
        mnemos_d1::initialize_heap(&AHEAP_BUF);
    }

    let mut p = unsafe { d1_pac::Peripherals::steal() };
    let uart = unsafe { kernel_uart(&mut p.CCU, &mut p.GPIO, p.UART0) };
    let spim = unsafe { kernel_spim1(p.SPI_DBI, &mut p.CCU, &mut p.GPIO) };
    let i2c0 = unsafe { twi::I2c0::lichee_rv_dock(p.TWI2, &mut p.CCU, &mut p.GPIO) };
    let timers = Timers::new(p.TIMER);
    let dmac = Dmac::new(p.DMAC, &mut p.CCU);
    let plic = Plic::new(p.PLIC);

    let d1 = D1::initialize(timers, uart, spim, dmac, plic, i2c0).unwrap();

    p.GPIO.pc_cfg0.modify(|_r, w| {
        w.pc1_select().output();
        w
    });
    p.GPIO.pc_dat.modify(|_r, w| {
        w.pc_dat().variant(0b0000_0010);
        w
    });

    // d1.initialize_sharp_display();

    for i in 0..4 {
        d1.kernel.initialize(async move {
            d1.kernel.sleep(Duration::from_secs(i * 3)).await;
            gui_demo(d1.kernel).await
        }).unwrap();
    }

    let _sharp_display = d1
        .kernel
        .initialize(SharpDisplay::register(d1.kernel))
        .expect("failed to spawn SHARP display driver");

    // Initialize LED loop
    d1.kernel
        .initialize(async move {
            loop {
                p.GPIO.pc_dat.modify(|_r, w| {
                    w.pc_dat().variant(0b0000_0010);
                    w
                });
                d1.kernel.sleep(Duration::from_millis(250)).await;
                p.GPIO.pc_dat.modify(|_r, w| {
                    w.pc_dat().variant(0b0000_0000);
                    w
                });
                d1.kernel.sleep(Duration::from_millis(250)).await;
            }
        })
        .unwrap();

    d1.run()
}




#[tracing::instrument(skip(k))]
async fn gui_demo(k: &'static Kernel) {
    let mut window = EmbDisplayClient::from_registry(k).await;

    const MIN_X: u32 = 0;
    const MAX_X: u32 = 400;
    const MIN_Y: u32 = 0;
    const MAX_Y: u32 = 240;

    let mut orb = MonoChunk::allocate_mono(FrameLocSize {
        offset_x: 0,
        offset_y: 0,
        width: 50,
        height: 50,
    }).await;

    let style = PrimitiveStyleBuilder::new()
        .fill_color(BinaryColor::On)
        .build();
    let _ = Circle::new(Point::new(0, 0), 48)
        .draw_styled(&style, &mut orb);

    // orb = window.draw_mono(orb).await.unwrap();

    let mut go_up = false;
    let mut go_left = false;

    loop {
        let meta = orb.meta_mut();
        let now_x = meta.start_x();
        let now_y = meta.start_y();

        if go_left {
            if now_x == 0 {
                meta.set_start_x(now_x + 1);
                go_left = false;
            } else {
                meta.set_start_x(now_x - 1);
            }
        } else {
            if now_x + meta.width() >= MAX_X {
                meta.set_start_x(now_x - 1);
                go_left = true;
            } else {
                meta.set_start_x(now_x + 1);
            }
        }

        if go_up {
            if now_y == 0 {
                meta.set_start_y(now_y + 1);
                go_up = false;
            } else {
                meta.set_start_y(now_y - 1);
            }
        } else {
            if now_y + meta.height() >= MAX_Y {
                meta.set_start_y(now_y - 1);
                go_up = true;
            } else {
                meta.set_start_y(now_y + 1);
            }
        }

        tracing::info!("DRAW");
        orb = window.draw_mono(orb).await.unwrap();
        orb.invert_masked();
        tracing::info!("DRAWDONE");
        k.sleep(Duration::from_millis(25)).await;
        tracing::info!("DRAW INVERT");
        orb = window.draw_mono(orb).await.unwrap();
        orb.invert_masked();
        tracing::info!("DRAW INVERTDONE");
    }
}
