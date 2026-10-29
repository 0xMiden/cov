//! A user contract project tested the way the Miden protocol tests its MASM.
//!
//! The project is three MASM files under `asm/`: a kernel standing in for the
//! transaction kernel, a wallet library that reaches the kernel through
//! `syscall`, and a note library that `call`s into the wallet. Its tests write
//! short programs that use the libraries, run them with [`CodeExecutor`], a
//! copy of the protocol's helper of the same name, and check what the programs
//! leave on the stack.
//!
//! [`CodeExecutor`] runs every program through the collector's
//! [`CoverageExecutor`], so the tests are measured without any change to them.
//! To get the coverage of the project's MASM, from the workspace root:
//!
//! ```text
//! MIDEN_COV_DIR=$PWD/target/miden-cov cargo test -p miden-cov-example-wallet
//! miden-cov report
//! ```
//!
//! The directory is absolute because each test runs in its package's
//! directory, not in the workspace root where the report looks by default.

use std::{
    path::PathBuf,
    pin::pin,
    sync::{Arc, LazyLock},
    task::{Context, Poll, Waker},
};

use miden_assembly::{
    Assembler, DefaultSourceManager, Linkage, ModuleParser, Path, SourceManager,
    ast::{Module, ModuleKind},
    debuginfo::{SourceLanguage, Uri},
    package::Package,
};
use miden_core_lib::CoreLibrary;
use miden_cov_collector::CoverageExecutor;
use miden_processor::{
    DefaultHost, ExecutionError, ExecutionOptions, ExecutionOutput, Felt, HostLibrary,
    ProcessorState, ProgramExecutor, StackInputs,
    advice::{AdviceInputs, AdviceMutation},
    event::{EventError, EventName},
};

/// The event the wallet emits to ask the host for the price of an asset.
pub const PRICE_REQUEST: &str = "example::wallet::price_request";

/// The project's packages, assembled once per test binary.
static PROJECT: LazyLock<Project> = LazyLock::new(Project::assemble);

/// The assembled packages of the project.
struct Project {
    /// The kernel the libraries and the programs are assembled against.
    kernel: Arc<Package>,
    /// The `example::wallet` library.
    wallet: Arc<Package>,
    /// The `example::note` library, linked to the wallet.
    note: Arc<Package>,
}

impl Project {
    /// Assembles the packages from the MASM files of the project.
    fn assemble() -> Self {
        let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
        let parse = |kind, path, file| {
            ModuleParser::new(Some(kind))
                .parse_file(Some(Path::new(path)), asm_file(file), source_manager.clone())
                .expect("the project's MASM parses")
        };
        let no_support = Vec::<Box<Module>>::new;

        let kernel_module = parse(ModuleKind::Kernel, Path::KERNEL_PATH, "kernel.masm");
        let kernel: Arc<Package> = Assembler::new(source_manager.clone())
            .assemble_kernel("example-kernel", kernel_module, no_support())
            .expect("the kernel assembles")
            .into();
        let assembler = || {
            Assembler::with_kernel(source_manager.clone(), kernel.clone()).expect("kernel links")
        };

        let wallet_module = parse(ModuleKind::Library, "example::wallet", "wallet.masm");
        let wallet: Arc<Package> = assembler()
            .assemble_library("example-wallet", wallet_module, no_support())
            .expect("the wallet assembles")
            .into();

        let note_module = parse(ModuleKind::Library, "example::note", "note.masm");
        let note: Arc<Package> = assembler()
            .with_package(wallet.clone(), Linkage::Dynamic)
            .expect("the wallet links")
            .assemble_library("example-note", note_module, no_support())
            .expect("the note assembles")
            .into();

        Self { kernel, wallet, note }
    }
}

/// Returns the path of a MASM file of the project.
///
/// The path is absolute so that the debug info records where the file is, and
/// the coverage report can find it from any directory.
fn asm_file(name: &str) -> PathBuf {
    [env!("CARGO_MANIFEST_DIR"), "asm", name].iter().collect()
}

/// Runs MASM programs against the project's kernel and libraries.
pub struct CodeExecutor {
    /// The host the programs run in.
    host: DefaultHost,
    /// The initial operand stack.
    stack_inputs: StackInputs,
    /// The initial advice provider contents.
    advice_inputs: AdviceInputs,
}

impl CodeExecutor {
    /// Returns an executor whose host holds the core library, the project's
    /// packages, and a handler for the wallet's price requests that quotes
    /// asset `n` at `10 * n`.
    pub fn with_default_host() -> Self {
        let mut host = DefaultHost::default();
        host.load_library(&CoreLibrary::default()).expect("the core library loads");
        // Packages, never bare MAST forests: a forest carries no debug info,
        // so the coverage would lose the package and all its branch records.
        for package in [&PROJECT.kernel, &PROJECT.wallet, &PROJECT.note] {
            host.load_library(HostLibrary::from(package.clone()))
                .expect("the package loads");
        }
        host.register_handler(EventName::new(PRICE_REQUEST), Arc::new(quote_price))
            .expect("the handler registers");
        Self {
            host,
            stack_inputs: StackInputs::default(),
            advice_inputs: AdviceInputs::default(),
        }
    }

    /// Adds `advice_inputs` to the advice provider contents.
    pub fn extend_advice_inputs(mut self, advice_inputs: AdviceInputs) -> Self {
        self.advice_inputs.extend(advice_inputs);
        self
    }

    /// Sets the initial operand stack.
    pub fn stack_inputs(mut self, stack_inputs: StackInputs) -> Self {
        self.stack_inputs = stack_inputs;
        self
    }

    /// Assembles the program `code` and runs it.
    ///
    /// `code` may use the core library and the `example::wallet` and
    /// `example::note` modules.
    ///
    /// # Panics
    ///
    /// Panics if `code` does not assemble.
    pub fn run(mut self, code: &str) -> Result<ExecutionOutput, ExecutionError> {
        let source_manager: Arc<dyn SourceManager> = Arc::new(DefaultSourceManager::default());
        let source =
            source_manager.load(SourceLanguage::Masm, Uri::new("_user_code"), code.to_owned());
        let package = Assembler::with_kernel(source_manager, PROJECT.kernel.clone())
            .and_then(|assembler| {
                assembler.with_package(CoreLibrary::default().package(), Linkage::Dynamic)
            })
            .and_then(|assembler| assembler.with_package(PROJECT.wallet.clone(), Linkage::Dynamic))
            .and_then(|assembler| assembler.with_package(PROJECT.note.clone(), Linkage::Dynamic))
            .and_then(|assembler| assembler.assemble_program("example-test", source))
            .expect("the program assembles");

        let executor = <CoverageExecutor as ProgramExecutor>::new(
            self.stack_inputs,
            self.advice_inputs,
            ExecutionOptions::default(),
        )
        .map_err(ExecutionError::advice_error_no_context)?
        .with_debug_info(
            package.debug_info().ok().flatten().expect("the assembler emits debug info"),
        )
        .with_entrypoint_source_node(package.entrypoint_source_node());
        block_on(executor.execute(&package.unwrap_program(), &mut self.host))
    }
}

/// Answers a price request with the price of the asset on top of the stack.
fn quote_price(process: &ProcessorState) -> Result<Vec<AdviceMutation>, EventError> {
    // `emit` keeps the event id on top of the stack while the handler runs.
    let asset = process.get_stack_item(1);
    Ok(vec![AdviceMutation::extend_advice_stack_with([asset * Felt::from(10u32)])])
}

/// Polls `future` to completion on the current thread.
///
/// The executor's future never waits on anything with a default host, so a
/// no-op waker is enough.
fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let mut cx = Context::from_waker(Waker::noop());
    loop {
        if let Poll::Ready(output) = future.as_mut().poll(&mut cx) {
            return output;
        }
    }
}
