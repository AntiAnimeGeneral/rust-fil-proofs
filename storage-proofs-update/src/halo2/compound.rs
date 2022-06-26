use filecoin_hashers::{Domain, Hasher, PoseidonArity};
use halo2_proofs::{arithmetic::FieldExt, plonk::Error};
use rand::rngs::OsRng;
use storage_proofs_core::halo2::{
    create_batch_proof, create_proof, verify_batch_proof, verify_proof, CompoundProof, FieldProvingCurves, Halo2Keypair, Halo2Proof,
};

use crate::{
    constants::{
        partition_count, TreeDArity, TreeDDomain, TreeDHasher, TreeRDomain, TreeRHasher, SECTOR_SIZE_16_KIB,
        SECTOR_SIZE_16_MIB, SECTOR_SIZE_1_KIB, SECTOR_SIZE_2_KIB, SECTOR_SIZE_32_GIB,
        SECTOR_SIZE_32_KIB, SECTOR_SIZE_4_KIB, SECTOR_SIZE_512_MIB, SECTOR_SIZE_64_GIB,
        SECTOR_SIZE_8_KIB, SECTOR_SIZE_8_MIB,
    },
    halo2::circuit::{self, EmptySectorUpdateCircuit},
    EmptySectorUpdate,
};

macro_rules! impl_compound_proof {
    ($($sector_nodes:expr),*) => {
        $(
            impl<'a, F, U, V, W> CompoundProof<'a, F, $sector_nodes> for EmptySectorUpdate<F, U, V, W>
            where
                F: FieldExt + FieldProvingCurves,
                U: PoseidonArity<F>,
                V: PoseidonArity<F>,
                W: PoseidonArity<F>,
                TreeDArity: PoseidonArity<F>,
                TreeDHasher<F>: Hasher<Domain = TreeDDomain<F>>,
                TreeDDomain<F>: Domain<Field = F>,
                TreeRHasher<F>: Hasher<Domain = TreeRDomain<F>>,
                TreeRDomain<F>: Domain<Field = F>,
            {
                type Circuit = EmptySectorUpdateCircuit<F, U, V, W, $sector_nodes>;

                fn prove_partition_with_vanilla(
                    setup_params: &Self::SetupParams,
                    vanilla_pub_inputs: &Self::PublicInputs,
                    vanilla_partition_proof: &Self::Proof,
                    keypair: &Halo2Keypair<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                ) -> Result<Halo2Proof<<F as FieldProvingCurves>::Affine, Self::Circuit>, Error> {
                    assert_eq!(setup_params.sector_bytes >> 5, $sector_nodes as u64);

                    let pub_inputs =
                        circuit::PublicInputs::<F, $sector_nodes>::from(vanilla_pub_inputs.clone());

                    let pub_inputs_vec = pub_inputs.to_vec();

                    let priv_inputs =
                        circuit::PrivateInputs::<F, U, V, W, $sector_nodes>::from(vanilla_partition_proof.clone());

                    let circ = EmptySectorUpdateCircuit {
                        pub_inputs,
                        priv_inputs,
                    };

                    create_proof(&keypair, circ, &pub_inputs_vec, &mut OsRng)
                }

                fn prove_all_partitions_with_vanilla(
                    setup_params: &Self::SetupParams,
                    vanilla_pub_inputs: &Self::PublicInputs,
                    vanilla_proofs: &[Self::Proof],
                    keypair: &Halo2Keypair<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                ) -> Result<Vec<Halo2Proof<<F as FieldProvingCurves>::Affine, Self::Circuit>>, Error> {
                    assert_eq!(setup_params.sector_bytes >> 5, $sector_nodes as u64);
                    let partition_count = partition_count($sector_nodes);
                    assert_eq!(vanilla_proofs.len(), partition_count);

                    let mut vanilla_pub_inputs = vanilla_pub_inputs.clone();

                    vanilla_proofs
                        .iter()
                        .enumerate()
                        .map(|(k, partition_proof)| {
                            // The only public input field which should change is `k`.
                            vanilla_pub_inputs.k = k;
                            <Self as CompoundProof<'_, F, $sector_nodes>>::prove_partition_with_vanilla(
                                setup_params,
                                &vanilla_pub_inputs,
                                partition_proof,
                                keypair,
                            )
                        })
                        .collect()
                }

                fn batch_prove_all_partitions_with_vanilla(
                    setup_params: &Self::SetupParams,
                    vanilla_pub_inputs: &Self::PublicInputs,
                    vanilla_proofs: &[Self::Proof],
                    keypair: &Halo2Keypair<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                ) -> Result<Halo2Proof<<F as FieldProvingCurves>::Affine, Self::Circuit>, Error> {
                    assert_eq!(setup_params.sector_bytes >> 5, $sector_nodes as u64);

                    let partition_count = partition_count($sector_nodes);
                    assert_eq!(vanilla_proofs.len(), partition_count);

                    let mut circ_pub_inputs_vecs = Vec::with_capacity(partition_count);

                    let circs: Vec<Self::Circuit> = vanilla_proofs
                        .iter()
                        .cloned()
                        .enumerate()
                        .map(|(k, vanilla_proof)| {
                            // The only public input field which should change is `k`.
                            let mut vanilla_pub_inputs = vanilla_pub_inputs.clone();
                            vanilla_pub_inputs.k = k;

                            let pub_inputs = circuit::PublicInputs::from(vanilla_pub_inputs);

                            circ_pub_inputs_vecs.push(pub_inputs.to_vec());

                            let priv_inputs = circuit::PrivateInputs::from(vanilla_proof);

                            EmptySectorUpdateCircuit {
                                pub_inputs,
                                priv_inputs,
                            }
                        })
                        .collect();

                    create_batch_proof(keypair, &circs, &circ_pub_inputs_vecs, &mut OsRng)
                }

                fn verify_partition(
                    setup_params: &Self::SetupParams,
                    vanilla_pub_inputs: &Self::PublicInputs,
                    circ_proof: &Halo2Proof<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                    keypair: &Halo2Keypair<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                ) -> Result<(), Error> {
                    assert_eq!(setup_params.sector_bytes >> 5, $sector_nodes as u64);
                    let pub_inputs =
                        circuit::PublicInputs::<F, $sector_nodes>::from(vanilla_pub_inputs.clone());
                    let pub_inputs_vec = pub_inputs.to_vec();
                    verify_proof(keypair, circ_proof, &pub_inputs_vec)
                }

                fn verify_all_partitions(
                    setup_params: &Self::SetupParams,
                    vanilla_pub_inputs: &Self::PublicInputs,
                    circ_proofs: &[Halo2Proof<<F as FieldProvingCurves>::Affine, Self::Circuit>],
                    keypair: &Halo2Keypair<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                ) -> Result<(), Error> {
                    assert_eq!(setup_params.sector_bytes >> 5, $sector_nodes as u64);
                    let partition_count = partition_count($sector_nodes);
                    assert_eq!(circ_proofs.len(), partition_count);

                    let mut vanilla_pub_inputs = vanilla_pub_inputs.clone();

                    for (k, partition_proof) in circ_proofs.iter().enumerate() {
                        // The only public input field which should change is `k`.
                        vanilla_pub_inputs.k = k;
                        <Self as CompoundProof<'_, F, $sector_nodes>>::verify_partition(
                            setup_params,
                            &vanilla_pub_inputs,
                            partition_proof,
                            keypair,
                        )?;
                    }
                    Ok(())
                }

                fn batch_verify_all_partitions(
                    setup_params: &Self::SetupParams,
                    vanilla_pub_inputs: &Self::PublicInputs,
                    batch_proof: &Halo2Proof<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                    keypair: &Halo2Keypair<<F as FieldProvingCurves>::Affine, Self::Circuit>,
                ) -> Result<(), Error> {
                    assert_eq!(setup_params.sector_bytes >> 5, $sector_nodes as u64);

                    let partition_count = partition_count($sector_nodes);

                    let circ_pub_inputs_vecs: Vec<Vec<Vec<F>>> = (0..partition_count)
                        .map(|k| {
                            // The only public input field which should change is `k`.
                            let mut vanilla_pub_inputs = vanilla_pub_inputs.clone();
                            vanilla_pub_inputs.k = k;
                            circuit::PublicInputs::<F, $sector_nodes>::from(vanilla_pub_inputs)
                                .to_vec()
                        })
                        .collect();

                    verify_batch_proof(keypair, batch_proof, &circ_pub_inputs_vecs, &mut OsRng)
                }
            }
        )*
    }
}

impl_compound_proof!(
    SECTOR_SIZE_1_KIB,
    SECTOR_SIZE_2_KIB,
    SECTOR_SIZE_4_KIB,
    SECTOR_SIZE_8_KIB,
    SECTOR_SIZE_16_KIB,
    SECTOR_SIZE_32_KIB,
    SECTOR_SIZE_8_MIB,
    SECTOR_SIZE_16_MIB,
    SECTOR_SIZE_512_MIB,
    SECTOR_SIZE_32_GIB,
    SECTOR_SIZE_64_GIB
);