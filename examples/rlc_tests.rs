use halo2_base::{
    gates::{
        builder::{GateCircuitBuilder, GateThreadBuilder},
        GateChip, RangeChip, RangeInstructions,
    },
    halo2_proofs::{
        dev::MockProver,
        halo2curves::bn256::{Bn256, Fr, G1Affine},
        plonk::{create_proof, keygen_pk, keygen_vk, verify_proof, Error},
        poly::{
            commitment::ParamsProver,
            kzg::{
                commitment::{KZGCommitmentScheme, ParamsKZG},
                multiopen::{ProverSHPLONK, VerifierSHPLONK},
                strategy::SingleStrategy,
            },
        },
        transcript::{
            Blake2bRead, Blake2bWrite, Challenge255, TranscriptReadBuffer, TranscriptWriterBuffer,
        },
    },
    safe_types::GateInstructions,
    utils::{bit_length, ScalarField},
    AssignedValue, Context,
    QuantumCell::{Constant, Existing, Witness},
};
use itertools::Itertools;
use rand::{rngs::StdRng, SeedableRng};
use std::env::{set_var, var};
use test_log::test;

use axiom_eth::rlp::{
    builder::{FnSynthesize, RlcCircuitBuilder, RlcThreadBuilder},
    rlc::RlcChip,
};

const DEGREE: u32 = 10;

fn rlc_test_circuit<F: ScalarField>(
    mut builder: RlcThreadBuilder<F>,
    _inputs: Vec<F>,
    _len: usize,
) -> RlcCircuitBuilder<F, impl FnSynthesize<F>> {
    let ctx = builder.gate_builder.main(0);
    let inputs = ctx.assign_witnesses(_inputs.clone());
    let len = ctx.load_witness(F::from(_len as u64));

    // let range: RangeChip<F> = RangeChip::default(5);
    // let zero = ctx.load_constant(F::zero());
    // let one = ctx.load_constant(F::one());
    // range.check_less_than(ctx, zero, one, 5);

    let synthesize_phase1 = move |builder: &mut RlcThreadBuilder<F>, rlc: &RlcChip<F>| {
        // the closure captures the `inputs` variable
        println!("phase 1 synthesize begin*");
        let gate = GateChip::default();

        let (ctx_gate, ctx_rlc) = builder.rlc_ctx_pair();
        // println!("Rand val = {:?}", rlc.gamma());

        // let init_rand = load_gamma(ctx_rlc, *rlc.gamma());
        rlc.load_rlc_cache((ctx_gate, ctx_rlc), &gate, 1);
        let init_rand = rlc.gamma_pow_cached()[0];
        println!("The init rand = {:?}", init_rand.value());
        println!("rlc.gamma = {:?}", *rlc.gamma());

        let rand_plus1 = gate.add(ctx_gate, init_rand, Constant(F::one()));
        println!("The rand_plus1 = {:?}", rand_plus1.value());

        let rlc_trace = rlc.compute_rlc((ctx_gate, ctx_rlc), &gate, inputs, len);
        let rlc_val = *rlc_trace.rlc_val.value();
        let real_rlc = compute_rlc_acc(&_inputs[.._len], *rlc.gamma());
        assert_eq!(real_rlc, rlc_val);
    };

    RlcCircuitBuilder::new(builder, None, synthesize_phase1)
}

fn compute_rlc_acc<F: ScalarField>(msg: &[F], r: F) -> F {
    if msg.is_empty() {
        return F::from(0);
    }
    let mut rlc = msg[0];
    for val in msg.iter().skip(1) {
        rlc = rlc * r + val;
    }
    rlc
}

pub fn test_mock_rlc() {
    let k = DEGREE;
    let input_bytes = vec![
        1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6,
        7, 8, 0, 0, 0, 0, 0, 0, 0, 0,
    ]
    .into_iter()
    .map(|x| Fr::from(x as u64))
    .collect_vec();
    let len = 32;

    let circuit = rlc_test_circuit(RlcThreadBuilder::mock(), input_bytes, len);

    circuit.config(k as usize, Some(6));
    MockProver::run(k, &circuit, vec![]).unwrap().assert_satisfied();
}

// // #[test_case([0,0,0,0].map(Fr::from).to_vec(); "RLC([0,0,0,0]) = 0")]
pub fn test_rlc_chip_zero<F: ScalarField>(inputs: Vec<F>) {
    let mut builder = RlcThreadBuilder::mock();
    let ctx = builder.gate_builder.main(0);
    let inputs = ctx.assign_witnesses(inputs);
    let len = ctx.load_witness(F::from(inputs.len() as u64));

    let circuit = RlcCircuitBuilder::new(
        builder,
        None,
        move |builder: &mut RlcThreadBuilder<F>, rlc: &RlcChip<F>| {
            let gate = GateChip::default();
            let (ctx_gate, ctx_rlc) = builder.rlc_ctx_pair();
            let rlc_trace = rlc.compute_rlc((ctx_gate, ctx_rlc), &gate, inputs, len);
            let rlc_val = *rlc_trace.rlc_val.value();
            assert_eq!(rlc_val, F::from(0));
        },
    );

    circuit.config(DEGREE as usize, Some(6));
    MockProver::run(DEGREE, &circuit, vec![]).unwrap().assert_satisfied();
}

#[derive(PartialEq, Debug)]
pub enum RlcTestErrors {
    RlcValError,
    RlcValAError,
    RlcValBError,
    GammaPowError,
    DynamicRlcError,
}

// // #[test_case(([1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 8, 8, 16) => Ok(()) ; "Dynamic RLC test, var len 1")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 3, 8, 11) => Ok(()) ; "Dynamic RLC test, var len 2")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 5, 8, 11) => Err(RlcTestErrors::DynamicRlcError) ; "Dynamic RLC test, c_len too small")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 1, 8, 11) => Err(RlcTestErrors::RlcValError) ; "Dynamic RLC test, c_len too big")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 0, 8, 11) => Err(RlcTestErrors::RlcValError) ; "Dynamic RLC test, c_len too small 2")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 3, 6, 11) => Err(RlcTestErrors::RlcValError) ; "Dynamic RLC test, c_len too big 2")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 3, 10, 11) => Err(RlcTestErrors::DynamicRlcError) ; "Dynamic RLC test, c_len too small 3")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 3, 0, 11) => Err(RlcTestErrors::RlcValError) ; "Dynamic RLC test, c_len too big 3")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 2, 5, 7) => Ok(()) ; "Dynamic RLC test, a_len + b_len = c_len")]
// // #[test_case((Vec::<Fr>::new(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), 0, 8, 8) => Ok(()) ; "Dynamic RLC test, a_len = 0")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), Vec::<Fr>::new(), 3, 0, 3) => Ok(()) ; "Dynamic RLC test, b_len = 0")]
pub fn test_rlc_dynamic_var_len<F: ScalarField>(
    inputs: (Vec<F>, Vec<F>, u64, u64, u64),
) -> Result<(), RlcTestErrors> {
    let mut rlc_builder = RlcThreadBuilder::mock();
    let mut builder = GateThreadBuilder::mock();
    let ctx = builder.main(0);

    let a_b_joined = inputs
        .0
        .iter()
        .take(inputs.2 as usize)
        .chain(inputs.1.iter().take(inputs.3 as usize))
        .cloned();
    let a_b = ctx.assign_witnesses(a_b_joined.clone());
    let a_b_len = ctx.load_witness(F::from(inputs.4));

    let a_len = ctx.load_witness(F::from(inputs.2));
    let mut a = inputs.0.clone().iter().take(inputs.2 as usize).cloned().collect_vec();
    a.resize(inputs.4 as usize, F::from(0));
    let witness_a = ctx.assign_witnesses(a.clone());

    let b_len = ctx.load_witness(F::from(inputs.3));
    let mut b = inputs.1.clone().iter().take(inputs.3 as usize).cloned().collect_vec();
    b.resize(inputs.4 as usize, F::from(0));
    let witness_b = ctx.assign_witnesses(b.clone());

    let gate = GateChip::default();
    let (ctx_gate, ctx_rlc) = rlc_builder.rlc_ctx_pair();
    let rlc = RlcChip::new(F::from(2));
    let rlc_trace = rlc.compute_rlc((ctx_gate, ctx_rlc), &gate, a_b, a_b_len);
    let rlc_val = *rlc_trace.rlc_val.value();
    let real_rlc_val = compute_rlc_acc(&a_b_joined.collect_vec(), *rlc.gamma());

    if rlc_val != real_rlc_val {
        return Err(RlcTestErrors::RlcValError);
    }

    let rlc_trace_a = rlc.compute_rlc((ctx_gate, ctx_rlc), &gate, witness_a, a_len);
    let rlc_val_a = *rlc_trace_a.rlc_val.value();
    let real_rlc_val_a = compute_rlc_acc(&a[..inputs.2 as usize], *rlc.gamma());

    if rlc_val_a != real_rlc_val_a {
        return Err(RlcTestErrors::RlcValAError);
    }

    let rlc_trace_b = rlc.compute_rlc((ctx_gate, ctx_rlc), &gate, witness_b, b_len);
    let rlc_val_b = *rlc_trace_b.rlc_val.value();
    let real_rlc_val_b = compute_rlc_acc(&b[..inputs.3 as usize], *rlc.gamma());

    if rlc_val_b != real_rlc_val_b {
        return Err(RlcTestErrors::RlcValBError);
    }

    rlc.load_rlc_cache((ctx_gate, ctx_rlc), &gate, inputs.4 as usize);
    let gamma_pow = rlc.rlc_pow(ctx_gate, &gate, b_len, bit_length(inputs.4));
    let real_gamma_pow = rlc.gamma().pow_vartime([inputs.3]);

    if *gamma_pow.value() != real_gamma_pow {
        return Err(RlcTestErrors::GammaPowError);
    }

    if rlc_val != rlc_val_a * gamma_pow.value() + rlc_val_b {
        return Err(RlcTestErrors::DynamicRlcError);
    }

    builder.config(6, Some(9));
    let circuit = GateCircuitBuilder::mock(builder);
    MockProver::run(6, &circuit, vec![]).unwrap().assert_satisfied();
    Ok(())
}

// // #[test_case(([1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec()) ; "Dynamic RLC test, fixed len 1")]
// // #[test_case(([1, 2, 3].map(Fr::from).to_vec(), [1, 2, 3, 4, 5, 6, 7, 8].map(Fr::from).to_vec()) ; "Dynamic RLC test, fixed len 2")]
pub fn test_rlc_dynamic_fixed_len<F: ScalarField>(inputs: (Vec<F>, Vec<F>)) {
    let mut rlc_builder = RlcThreadBuilder::mock();
    let mut builder = GateThreadBuilder::mock();
    let ctx = builder.main(0);

    let a_b = ctx.assign_witnesses(inputs.0.iter().chain(inputs.1.iter()).cloned());
    let combined_len = inputs.0.len() as u64 + inputs.1.len() as u64;
    let a = ctx.assign_witnesses(inputs.0.clone());
    let b_len = ctx.load_witness(F::from(inputs.1.len() as u64));
    let b = ctx.assign_witnesses(inputs.1.clone());

    let gate = GateChip::default();
    let (ctx_gate, ctx_rlc) = rlc_builder.rlc_ctx_pair();
    let rlc = RlcChip::new(F::from(2));
    let rlc_trace = rlc.compute_rlc_fixed_len(ctx_rlc, a_b);
    let rlc_val = *rlc_trace.rlc_val.value();
    let real_rlc_val = compute_rlc_acc(
        &inputs.0.iter().chain(inputs.1.iter()).cloned().collect_vec(),
        *rlc.gamma(),
    );
    assert_eq!(rlc_val, real_rlc_val);

    let rlc_trace_a = rlc.compute_rlc_fixed_len(ctx_rlc, a);
    let rlc_val_a = *rlc_trace_a.rlc_val.value();
    let real_rlc_val_a = compute_rlc_acc(&inputs.0, *rlc.gamma());
    assert_eq!(rlc_val_a, real_rlc_val_a);

    let rlc_trace_b = rlc.compute_rlc_fixed_len(ctx_rlc, b);
    let rlc_val_b = *rlc_trace_b.rlc_val.value();
    let real_rlc_val_b = compute_rlc_acc(&inputs.1, *rlc.gamma());
    assert_eq!(rlc_val_b, real_rlc_val_b);

    rlc.load_rlc_cache((ctx_gate, ctx_rlc), &gate, combined_len as usize);
    let gamma_pow = rlc.rlc_pow(ctx_gate, &gate, b_len, bit_length(combined_len));
    let real_gamma_pow = rlc.gamma().pow_vartime([inputs.1.len() as u64]);
    assert_eq!(*gamma_pow.value(), real_gamma_pow);
    assert_eq!(rlc_val, rlc_val_a * gamma_pow.value() + rlc_val_b);

    builder.config(6, Some(9));
    let circuit = GateCircuitBuilder::mock(builder);
    MockProver::run(6, &circuit, vec![]).unwrap().assert_satisfied()
}

pub fn test_rlc() -> Result<(), Error> {
    let k = DEGREE;
    let input_bytes = vec![
        1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6, 7, 8, 1, 2, 3, 4, 5, 6,
        7, 8, 0, 0, 0, 0, 0, 0, 0, 0,
    ]
    .into_iter()
    .map(|x| Fr::from(x as u64))
    .collect_vec();
    let len = 32;

    let mut rng = StdRng::from_seed([0u8; 32]);
    let params = ParamsKZG::<Bn256>::setup(k, &mut rng);
    let circuit = rlc_test_circuit(RlcThreadBuilder::keygen(), input_bytes.clone(), len);
    circuit.config(k as usize, Some(6));

    println!("vk gen started");
    let vk = keygen_vk(&params, &circuit)?;
    println!("vk gen done");
    let pk = keygen_pk(&params, vk, &circuit)?;
    println!("pk gen done");
    println!();
    println!("==============STARTING PROOF GEN===================");
    let break_points = circuit.break_points.take();
    drop(circuit);
    let circuit = rlc_test_circuit(RlcThreadBuilder::prover(), input_bytes, len);
    *circuit.break_points.borrow_mut() = break_points;

    let mut transcript = Blake2bWrite::<_, _, Challenge255<_>>::init(vec![]);
    create_proof::<
        KZGCommitmentScheme<Bn256>,
        ProverSHPLONK<'_, Bn256>,
        Challenge255<G1Affine>,
        _,
        Blake2bWrite<Vec<u8>, G1Affine, Challenge255<G1Affine>>,
        _,
    >(&params, &pk, &[circuit], &[&[]], rng, &mut transcript)?;
    let proof = transcript.finalize();
    println!("proof gen done");
    let verifier_params = params.verifier_params();
    let strategy = SingleStrategy::new(verifier_params);
    let mut transcript = Blake2bRead::<_, _, Challenge255<_>>::init(&proof[..]);
    verify_proof::<
        KZGCommitmentScheme<Bn256>,
        VerifierSHPLONK<'_, Bn256>,
        Challenge255<G1Affine>,
        Blake2bRead<&[u8], G1Affine, Challenge255<G1Affine>>,
        SingleStrategy<'_, Bn256>,
    >(verifier_params, pk.get_vk(), strategy, &[&[]], &mut transcript)
    .unwrap();
    println!("verify done");
    Ok(())
}

/// Returns challenge value as a AssignedValue
///
/// `ctx_rlc` should be the RLC context
///
/// gamma should be the challenge value as a field element
///
/// Copied from the corresponding private function of RlcChip
fn load_gamma<F: ScalarField>(ctx_rlc: &mut Context<F>, gamma: F) -> AssignedValue<F> {
    ctx_rlc.assign_region_last([Constant(F::one()), Constant(F::zero()), Witness(gamma)], [0])
}

fn main() {
    set_var("LOOKUP_BITS", 5.to_string());

    env_logger::init();

    test_rlc();
}