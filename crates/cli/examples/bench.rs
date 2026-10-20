//! Measures the per-cycle cost of collecting coverage.
//!
//! Runs the `nprime` and `blake3_1to1` fixtures several ways and prints the
//! cycles per second of each: the processor alone as the floor, the collector
//! fed by the engine's stepper, and the collector fed by the engine's debug
//! executor. Build with `--release`; the dev profile says nothing about the
//! real cost.
//!
//! ```sh
//! cargo run --release -p miden-cov-cli --example bench
//! ```

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use miden_assembly::{Assembler, Linkage};
use miden_core_lib::CoreLibrary;
use miden_cov_collector::Collector;
use miden_debug_engine::{
    ExecutionConfig, Executor, Stepper,
    debug_types::DefaultSourceManager,
    processor::{
        DefaultHost, ExecutionOptions, FastProcessor, Felt, HostLibrary, StackInputs,
        advice::{AdviceInputs, AdviceStack},
    },
};
use miden_mast_package::Package;

/// A fixture with its inputs.
struct Program {
    name: &'static str,
    stack: Vec<u64>,
    advice: Vec<u64>,
}

impl Program {
    fn assemble(&self) -> Arc<Package> {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/examples")
            .join(format!("{}.masm", self.name));
        Arc::from(
            Assembler::default()
                .with_package(CoreLibrary::default().package(), Linkage::Dynamic)
                .unwrap()
                .assemble_program(self.name, path)
                .unwrap(),
        )
    }

    fn stack_inputs(&self) -> StackInputs {
        StackInputs::new(&felts(&self.stack)).unwrap()
    }

    fn advice_inputs(&self) -> AdviceInputs {
        AdviceInputs::default().with_stack(AdviceStack::from(felts(&self.advice)))
    }

    fn processor(&self) -> FastProcessor {
        FastProcessor::new_with_options(
            self.stack_inputs(),
            self.advice_inputs(),
            ExecutionOptions::default(),
        )
        .unwrap()
    }
}

fn felts(values: &[u64]) -> Vec<Felt> {
    values.iter().map(|value| Felt::new(*value).unwrap()).collect()
}

fn core_host() -> DefaultHost {
    let mut host = DefaultHost::default();
    host.load_library(HostLibrary::from(CoreLibrary::default().package())).unwrap();
    host
}

/// The processor alone, no stepping. Returns the cycle count from a stepped
/// run, since the output does not carry it.
fn floor(program: &Program, package: &Arc<Package>) {
    let mut host = core_host();
    let debug_info = package.debug_info().unwrap().unwrap();
    program
        .processor()
        .execute_with_package_debug_info_sync(&package.unwrap_program(), &debug_info, &mut host)
        .unwrap();
}

/// The collector fed by the engine's stepper.
fn stepper(program: &Program, package: &Arc<Package>) -> u64 {
    let mut host = core_host();
    let mut stepper = Stepper::for_package(program.processor(), package.clone()).unwrap();
    let mut collector = Collector::new();
    stepper.run_sync(&mut host, |cycle| collector.on_cycle(cycle)).unwrap();
    collector.cycles()
}

/// The collector fed by the engine's debug executor stepped to the end.
fn engine(program: &Program, package: &Arc<Package>) -> u64 {
    let mut executor = Executor::from_config(ExecutionConfig {
        inputs: program.stack_inputs(),
        advice_inputs: program.advice_inputs(),
        options: ExecutionOptions::default(),
    });
    executor.with_package(CoreLibrary::default().package()).unwrap();
    let mut exec = executor.into_debug(package.clone(), Arc::new(DefaultSourceManager::default()));
    let mut collector = Collector::new();
    while !exec.stopped {
        exec.step_with(|cycle| collector.on_cycle(cycle)).unwrap();
    }
    collector.cycles()
}

/// Runs `f` `rounds` times and returns the best wall time.
fn best_of<T>(rounds: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut best: Option<(Duration, T)> = None;
    for _ in 0..rounds {
        let started = Instant::now();
        let value = f();
        let elapsed = started.elapsed();
        if best.as_ref().is_none_or(|(d, _)| elapsed < *d) {
            best = Some((elapsed, value));
        }
    }
    best.unwrap()
}

fn report(label: &str, cycles: u64, elapsed: Duration) {
    let per_second = cycles as f64 / elapsed.as_secs_f64();
    println!(
        "  {label:<10} {:>10.1} ms  {:>10.0} cycles/s  ({:.0} ns/cycle)",
        elapsed.as_secs_f64() * 1e3,
        per_second,
        1e9 / per_second
    );
}

fn main() {
    let rounds: usize = std::env::args().nth(1).and_then(|n| n.parse().ok()).unwrap_or(5);
    let programs = [
        Program {
            name: "nprime",
            stack: vec![50],
            advice: vec![],
        },
        Program {
            name: "blake3_1to1",
            stack: vec![u32::MAX.into(); 8],
            advice: vec![100],
        },
    ];
    for program in &programs {
        let package = program.assemble();
        let (stepper_time, cycles) = best_of(rounds, || stepper(program, &package));
        let (engine_time, engine_cycles) = best_of(rounds, || engine(program, &package));
        assert_eq!(cycles, engine_cycles, "{}: cycle counts differ", program.name);
        let (floor_time, ()) = best_of(rounds, || floor(program, &package));
        println!("{} ({cycles} cycles, best of {rounds})", program.name);
        report("processor", cycles, floor_time);
        report("stepper", cycles, stepper_time);
        report("engine", cycles, engine_time);
    }
}
