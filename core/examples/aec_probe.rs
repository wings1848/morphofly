//! 临时探针：直接驱动 sonora，观察 AEC3 内部统计（诊断完删除）。
use sonora::config::EchoCanceller;
use sonora::{AudioProcessing, Config, StreamConfig};

const RATE: u32 = 16_000;
const BLK: usize = 160;

struct Lcg(u64);
impl Lcg {
    fn next_unit(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        let bits = (self.0 >> 33) as u32;
        (bits as f32 / (u32::MAX >> 1) as f32) * 2.0 - 1.0
    }
}

fn white(len: usize, seed: u64) -> Vec<f32> {
    let mut rng = Lcg(seed);
    (0..len).map(|_| 0.2 * rng.next_unit()).collect()
}

fn speech_like(len: usize, seed: u64) -> Vec<f32> {
    let mut rng = Lcg(seed);
    let mut out = Vec::with_capacity(len);
    for n in 0..len {
        let t = n as f32 / RATE as f32;
        let am = 0.55 + 0.45 * (2.0 * std::f32::consts::PI * 3.1 * t).sin();
        let mut v = 0.0f32;
        for (i, f) in [233.0f32, 521.0, 879.0, 1_337.0, 2_101.0].iter().enumerate() {
            let phase = i as f32 * 0.7;
            v += (2.0 * std::f32::consts::PI * f * t + phase).sin() / (i as f32 + 2.0);
        }
        v += 0.35 * rng.next_unit();
        out.push(v * am * 0.25);
    }
    out
}

fn energy(x: &[f32]) -> f64 {
    x.iter().map(|&v| f64::from(v) * f64::from(v)).sum::<f64>() / x.len() as f64
}

fn trial(name: &str, render: Vec<f32>, delay: usize, gain: f32, use_i16: bool) {
    let len = render.len();
    let mut capture = vec![0.0f32; len];
    for n in 0..len {
        capture[n] = if n >= delay { gain * render[n - delay] } else { 0.0 };
    }

    let cfg = Config {
        echo_canceller: Some(EchoCanceller::default()),
        ..Default::default()
    };
    let stream = StreamConfig::new(RATE, 1);
    let mut apm = AudioProcessing::builder()
        .config(cfg)
        .capture_config(stream)
        .render_config(stream)
        .build();

    let mut out = capture.clone();
    let blocks = len / BLK;
    for i in 0..blocks {
        let r = &render[i * BLK..(i + 1) * BLK];
        let mut rout = [0.0f32; BLK];
        let src = [r];
        let mut dst = [&mut rout[..]];
        apm.process_render_f32(&src, &mut dst).unwrap();

        let c = &mut out[i * BLK..(i + 1) * BLK];
        if use_i16 {
            let in_i16: Vec<i16> = c
                .iter()
                .map(|&v| (v.clamp(-1.0, 1.0) * 32768.0).round().clamp(-32768.0, 32767.0) as i16)
                .collect();
            let mut o = in_i16.clone();
            apm.process_capture_i16(&in_i16, &mut o).unwrap();
            for (d, s) in c.iter_mut().zip(o.iter()) {
                *d = f32::from(*s) / 32768.0;
            }
        } else {
            let blk = {
                let mut b = [0.0f32; BLK];
                b.copy_from_slice(c);
                b
            };
            let src = [&blk[..]];
            let mut dst = [&mut *c];
            apm.process_capture_f32(&src, &mut dst).unwrap();
        }
    }
    let st = apm.statistics();
    let tail = RATE as usize * 2;
    let in_t = &capture[len - tail..];
    let out_t = &out[len - tail..];
    eprintln!(
        "{name}: tailERLE={:.1}dB delay={:?}/{:?} erle={:?} erl={:?} div={:?} i16={use_i16}",
        10.0 * (energy(in_t) / energy(out_t)).log10(),
        st.delay_ms,
        st.delay_median_ms,
        st.echo_return_loss_enhancement,
        st.echo_return_loss,
        st.divergent_filter_fraction,
    );
}

fn main() {
    let n5 = RATE as usize * 5;
    let n10 = RATE as usize * 10;
    trial("white/d20ms/g0.5/f32", white(n10, 1), 320, 0.5, false);
    trial("white/d20ms/g0.5/i16", white(n10, 1), 320, 0.5, true);
    trial("white/d0/f32", white(n10, 2), 0, 0.5, false);
    trial("white/d20ms/g0.05/f32", white(n10, 3), 320, 0.05, false);
    trial("speech/d20ms/g0.6/f32", speech_like(n5, 42), 320, 0.6, false);
    trial("speech10/d20ms/g0.6/f32", speech_like(n10, 42), 320, 0.6, false);
    trial("speech10/d100ms/g0.6/f32", speech_like(n10, 43), 1600, 0.6, false);
}
