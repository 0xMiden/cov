//! Tests of the wallet and note libraries.

use miden_cov_example_wallet::CodeExecutor;
use miden_processor::{
    ExecutionError, ExecutionOutput, Felt, StackInputs, Word, advice::AdviceInputs,
    operation::OperationError,
};

/// Returns the top `n` elements of the output stack as integers.
fn top(output: &ExecutionOutput, n: usize) -> Vec<u64> {
    output.stack.get_num_elements(n).iter().map(Felt::as_canonical_u64).collect()
}

#[test]
fn deposit_many_adds_every_deposit() {
    let output = CodeExecutor::with_default_host()
        .run(
            "
            use miden::core::sys
            use example::wallet

            begin
                push.100 push.5 push.3
                exec.wallet::deposit_many
                exec.sys::truncate_stack
            end
            ",
        )
        .unwrap();

    assert_eq!(top(&output, 1), [115]);
}

#[test]
fn deposit_of_zero_keeps_the_balance() {
    let output = CodeExecutor::with_default_host()
        .run(
            "
            use miden::core::sys
            use example::wallet

            begin
                push.100 push.0
                exec.wallet::deposit
                exec.sys::truncate_stack
            end
            ",
        )
        .unwrap();

    assert_eq!(top(&output, 1), [100]);
}

#[test]
fn withdraw_beyond_the_balance_fails() {
    let result = CodeExecutor::with_default_host().run(
        "
        use miden::core::sys
        use example::wallet

        begin
            push.10 push.8
            exec.wallet::withdraw
            exec.sys::truncate_stack
        end
        ",
    );

    match result {
        Err(ExecutionError::OperationError {
            err: OperationError::FailedAssertion { err_msg: Some(msg), .. },
            ..
        }) => assert_eq!(&*msg, "insufficient funds"),
        other => panic!("expected the insufficient funds assertion, got {other:?}"),
    }
}

#[test]
fn spending_limit_is_read_from_the_advice_map() {
    let key = Word::from([7u32, 7, 7, 7]);
    let advice = AdviceInputs::default().with_map([(key, vec![Felt::from(500u32)])]);
    let output = CodeExecutor::with_default_host()
        .extend_advice_inputs(advice)
        .run(&format!(
            "
            use miden::core::sys
            use example::wallet

            begin
                push.{key}
                exec.wallet::spending_limit
                exec.sys::truncate_stack
            end
            "
        ))
        .unwrap();

    assert_eq!(top(&output, 1), [500]);
}

#[test]
fn value_of_uses_the_price_quoted_by_the_host() {
    let output = CodeExecutor::with_default_host()
        .run(
            "
            use miden::core::sys
            use example::wallet

            begin
                push.4 push.7
                exec.wallet::value_of
                exec.sys::truncate_stack
            end
            ",
        )
        .unwrap();

    // The host quotes asset 7 at 70.
    assert_eq!(top(&output, 1), [280]);
}

#[test]
fn note_pays_out_of_the_balance_with_the_fee() {
    let amount = Felt::from(20u32);
    let balance = Felt::from(100u32);
    let output = CodeExecutor::with_default_host()
        .stack_inputs(StackInputs::new(&[amount, balance]).unwrap())
        .run(
            "
            use example::note

            begin
                exec.note::pay
            end
            ",
        )
        .unwrap();

    assert_eq!(top(&output, 2), [77, 0]);
}
