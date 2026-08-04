//! Pipeline latency benchmark CLI tool.
//!
//! Measures the end-to-end performance of Renderd's data plane primitives
//! including fragment header codec throughput, sliding-window reassembly
//! throughput, and clock statistics calculation speed.

use std::time::Instant;

use clap::{Parser, Subcommand};
use renderd_clock::RollingStats;
use renderd_frame::{FragmentHeader, ReassemblyBuffer, FLAG_FIRST_FRAG, FLAG_LAST_FRAG};

#[derive(Parser, Debug)]
#[command(
    name = "latency-bench",
    author,
    version,
    about = "Renderd Pipeline Latency Benchmark CLI Tool"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Benchmark datagram fragment header encoding and reassembly buffer performance.
    Framing {
        /// Number of iterations.
        #[arg(short, long, default_value_t = 100_000)]
        iterations: usize,
    },
    /// Benchmark NTP/PTP rolling clock offset and percentile statistics calculation.
    Clock {
        /// Number of iterations.
        #[arg(short, long, default_value_t = 100_000)]
        iterations: usize,
    },
    /// Print pipeline target budget report.
    Budget,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Framing { iterations } => {
            println!("--- Running Datagram Framing Benchmark ({iterations} iterations) ---");
            let mut header_buf = [0u8; 16];
            let header = FragmentHeader {
                frame_id: 1,
                frag_id: 0,
                frag_total: 10,
                flags: FLAG_FIRST_FRAG,
                pts_offset_us: 1500,
            };

            let start = Instant::now();
            for _ in 0..iterations {
                header.encode(&mut header_buf).unwrap();
                let _decoded = FragmentHeader::decode(&header_buf).unwrap();
            }
            let elapsed = start.elapsed();
            // iterations is always > 0 in this context (default = 100_000).
            // The cast is safe for any realistic benchmark iteration count (<= u32::MAX).
            #[allow(clippy::cast_possible_truncation)]
            let per_op = elapsed / (iterations as u32);
            println!("Codec throughput: {iterations} encode/decode cycles in {elapsed:.2?}");
            println!("Average latency per encode/decode cycle: {per_op:.2?}");

            let mut reasm_stats = RollingStats::<100>::new();
            let payload = bytes::Bytes::from_static(&[0x55u8; 1200]);
            let reasm_start = Instant::now();
            for i in 0..iterations {
                let op_start = Instant::now();
                let mut buffer = ReassemblyBuffer::new(16);
                for f in 0u16..10 {
                    let h = FragmentHeader {
                        frame_id: i as u64,
                        frag_id: f,
                        frag_total: 10,
                        flags: if f == 0 {
                            FLAG_FIRST_FRAG
                        } else if f == 9 {
                            FLAG_LAST_FRAG
                        } else {
                            0
                        },
                        pts_offset_us: 0,
                    };
                    let _ = buffer.insert(h, payload.clone());
                }
                #[allow(clippy::cast_possible_truncation)]
                reasm_stats.push(op_start.elapsed().as_nanos() as u64);
            }
            let reasm_elapsed = reasm_start.elapsed();
            println!("\nReassembly 10-fragment frame insertion benchmark:");
            println!("Total time: {reasm_elapsed:.2?}");
            // Converting u64 nanoseconds to f64 microseconds. Precision loss is acceptable
            // for a human-readable benchmark printout; no arithmetic correctness depends on it.
            #[allow(clippy::cast_precision_loss)]
            let mean_us = reasm_stats.mean() / 1000.0;
            #[allow(clippy::cast_precision_loss)]
            let p95_us = reasm_stats.percentile(95.0) as f64 / 1000.0;
            println!("Mean frame reassembly time: {mean_us:.2} µs");
            println!("p95 frame reassembly time: {p95_us:.2} µs");
        }
        Commands::Clock { iterations } => {
            println!("--- Running Clock & Rolling Stats Benchmark ({iterations} iterations) ---");
            let mut stats = RollingStats::<1000>::new();
            let start = Instant::now();
            for i in 0..iterations {
                #[allow(clippy::cast_possible_truncation)]
                stats.push((i % 1000) as u64 * 10);
            }
            let elapsed = start.elapsed();
            println!("Recorded {iterations} rolling samples in {elapsed:.2?}");
            println!("Calculated Mean: {:.2}", stats.mean());
            println!("Calculated Variance: {:.2}", stats.variance());
            println!("Calculated p99: {}", stats.percentile(99.0));
        }
        Commands::Budget => {
            println!("============================================================");
            println!("        RENDERD PIPELINE LATENCY TARGET BUDGET (RFC-0002 §19)");
            println!("============================================================");
            println!(" Stage                      Target (p50)    Target (p99)");
            println!(" -----------------------------------------------------------");
            println!(" 1. ScreenCaptureKit         ~2 ms           ~3 ms");
            println!(" 2. VideoToolbox HW Encode   ~7 ms          ~11 ms");
            println!(" 3. QUIC Framing + Send      ~0.5 ms         ~1 ms");
            println!(" 4. Network (Gigabit LAN)    ~0.5 ms         ~1.5 ms");
            println!(" 5. QUIC Recv + Reassembly   ~0.3 ms         ~0.8 ms");
            println!(" 6. D3D12 HW Video Decode    ~2 ms           ~4 ms");
            println!(" 7. D3D12 Render + Present   ~1.5 ms         ~2.5 ms");
            println!(" 8. Display scanout (phsync)  ~2 ms          ~4 ms");
            println!(" -----------------------------------------------------------");
            println!(" TOTAL (1080p60):             ~16 ms         ~28 ms");
            println!(" TARGET: ≤ 30 ms glass-to-glass at 1080p60 on Gigabit LAN");
            println!("============================================================");
        }
    }
}
