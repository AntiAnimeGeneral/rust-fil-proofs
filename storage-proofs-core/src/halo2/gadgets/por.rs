use fil_halo2_gadgets::{boolean::AssignedBit, WitnessOrCopy};
use filecoin_hashers::{Halo2Hasher, HashInstructions, PoseidonArity};
use generic_array::typenum::U0;
use halo2_proofs::{
    arithmetic::FieldExt,
    circuit::{AssignedCell, Layouter, Value},
    plonk::Error,
};

use crate::halo2::gadgets::insert::{InsertChip, InsertConfig};

pub struct MerkleChip<H, U, V = U0, W = U0>
where
    H: Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
    H::Field: FieldExt,
    U: PoseidonArity<H::Field>,
    V: PoseidonArity<H::Field>,
    W: PoseidonArity<H::Field>,
{
    base_hasher: <H as Halo2Hasher<U>>::Chip,
    base_insert: InsertChip<H::Field, U>,
    sub_hasher_insert: Option<(<H as Halo2Hasher<V>>::Chip, InsertChip<H::Field, V>)>,
    top_hasher_insert: Option<(<H as Halo2Hasher<W>>::Chip, InsertChip<H::Field, W>)>,
}

impl<H, U, V, W> MerkleChip<H, U, V, W>
where
    H: Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
    H::Field: FieldExt,
    U: PoseidonArity<H::Field>,
    V: PoseidonArity<H::Field>,
    W: PoseidonArity<H::Field>,
{
    pub fn with_subchips(
        base_hasher: <H as Halo2Hasher<U>>::Chip,
        base_insert: InsertChip<H::Field, U>,
        sub_hasher_insert: Option<(<H as Halo2Hasher<V>>::Chip, InsertChip<H::Field, V>)>,
        top_hasher_insert: Option<(<H as Halo2Hasher<W>>::Chip, InsertChip<H::Field, W>)>,
    ) -> Self {
        if V::to_usize() == 0 {
            assert!(sub_hasher_insert.is_none());
        } else {
            assert!(sub_hasher_insert.is_some());
        };
        if W::to_usize() == 0 {
            assert!(top_hasher_insert.is_none());
        } else {
            assert!(top_hasher_insert.is_some());
        };
        MerkleChip {
            base_hasher,
            base_insert,
            sub_hasher_insert,
            top_hasher_insert,
        }
    }

    pub fn compute_root(
        &self,
        layouter: impl Layouter<H::Field>,
        challenge_bits: &[AssignedBit<H::Field>],
        leaf: Value<H::Field>,
        path: &[Vec<Value<H::Field>>],
    ) -> Result<AssignedCell<H::Field, H::Field>, Error> {
        self.compute_root_inner(layouter, challenge_bits, WitnessOrCopy::Witness(leaf), path)
    }

    pub fn copy_leaf_compute_root(
        &self,
        layouter: impl Layouter<H::Field>,
        challenge_bits: &[AssignedBit<H::Field>],
        leaf: &AssignedCell<H::Field, H::Field>,
        path: &[Vec<Value<H::Field>>],
    ) -> Result<AssignedCell<H::Field, H::Field>, Error> {
        self.compute_root_inner(
            layouter,
            challenge_bits,
            WitnessOrCopy::Copy(leaf.clone()),
            path,
        )
    }

    #[allow(unused_assignments)]
    #[allow(clippy::unwrap_used)]
    fn compute_root_inner(
        &self,
        mut layouter: impl Layouter<H::Field>,
        challenge_bits: &[AssignedBit<H::Field>],
        leaf: WitnessOrCopy<H::Field, H::Field>,
        path: &[Vec<Value<H::Field>>],
    ) -> Result<AssignedCell<H::Field, H::Field>, Error> {
        let base_arity = U::to_usize();
        let sub_arity = V::to_usize();
        let top_arity = W::to_usize();

        let base_arity_bit_len = base_arity.trailing_zeros();
        let sub_arity_bit_len = sub_arity.trailing_zeros();
        let top_arity_bit_len = top_arity.trailing_zeros();

        let base_path_len = if top_arity > 0 {
            path.len() - 2
        } else if sub_arity > 0 {
            path.len() - 1
        } else {
            path.len()
        };

        let mut cur = None;
        let mut height = 0;
        let mut path_values = path.iter();
        let mut challenge_bits = challenge_bits.iter();

        for _ in 0..base_path_len {
            let siblings = path_values.next().expect("no path elements remaining");

            assert_eq!(
                siblings.len(),
                base_arity - 1,
                "path element has incorrect number of siblings"
            );

            let index_bits: Vec<AssignedBit<H::Field>> = (0..base_arity_bit_len)
                .map(|_| {
                    challenge_bits
                        .next()
                        .expect("no challenge bits remaining")
                        .clone()
                })
                .collect();

            let preimage = if height == 0 {
                match leaf {
                    WitnessOrCopy::Witness(ref leaf) => self.base_insert.witness_insert(
                        layouter.namespace(|| format!("base insert (height {})", height)),
                        siblings,
                        leaf,
                        &index_bits,
                    )?,
                    WitnessOrCopy::Copy(ref leaf) => self.base_insert.copy_insert(
                        layouter.namespace(|| format!("base insert (height {})", height)),
                        siblings,
                        leaf,
                        &index_bits,
                    )?,
                    _ => unimplemented!(),
                }
            } else {
                self.base_insert.copy_insert(
                    layouter.namespace(|| format!("base insert (height {})", height)),
                    siblings,
                    &cur.take().unwrap(),
                    &index_bits,
                )?
            };

            let digest = self.base_hasher.hash(
                layouter.namespace(|| format!("base hash siblings (height {})", height)),
                &preimage,
            )?;

            cur = Some(digest);
            height += 1;
        }

        if sub_arity > 0 {
            let siblings = path_values.next().expect("no path elements remaining");

            assert_eq!(
                siblings.len(),
                sub_arity - 1,
                "path element has incorrect number of siblings"
            );

            let index_bits: Vec<AssignedBit<H::Field>> = (0..sub_arity_bit_len)
                .map(|_| {
                    challenge_bits
                        .next()
                        .expect("no challenge bits remaining")
                        .clone()
                })
                .collect();

            let (sub_hasher, sub_insert) = self.sub_hasher_insert.as_ref().unwrap();

            let preimage = sub_insert.copy_insert(
                layouter.namespace(|| format!("insert (height {})", height)),
                siblings,
                &cur.take().unwrap(),
                &index_bits,
            )?;

            let digest = sub_hasher.hash(
                layouter.namespace(|| format!("merkle proof hash (height {})", height)),
                &preimage,
            )?;

            cur = Some(digest);
            height += 1;
        }

        if top_arity > 0 {
            let siblings = path_values.next().expect("no path elements remaining");

            assert_eq!(
                siblings.len(),
                top_arity - 1,
                "path element has incorrect number of siblings"
            );

            let index_bits: Vec<AssignedBit<H::Field>> = (0..top_arity_bit_len)
                .map(|_| {
                    challenge_bits
                        .next()
                        .expect("no challenge bits remaining")
                        .clone()
                })
                .collect();

            let (top_hasher, top_insert) = self.top_hasher_insert.as_ref().unwrap();

            let preimage = top_insert.copy_insert(
                layouter.namespace(|| format!("insert (height {})", height)),
                siblings,
                &cur.take().unwrap(),
                &index_bits,
            )?;

            let digest = top_hasher.hash(
                layouter.namespace(|| format!("merkle proof hash (height {})", height)),
                &preimage,
            )?;

            cur = Some(digest);
            height += 1;
        }

        Ok(cur.unwrap())
    }
}

pub fn empty_path<F, U, V, W, const NUM_LEAFS: usize>() -> Vec<Vec<Value<F>>>
where
    F: FieldExt,
    U: PoseidonArity<F>,
    V: PoseidonArity<F>,
    W: PoseidonArity<F>,
{
    let base_arity = U::to_usize();
    let sub_arity = V::to_usize();
    let top_arity = W::to_usize();

    let challenge_bit_len = NUM_LEAFS.trailing_zeros() as usize;

    let base_height = {
        let mut base_challenge_bit_len = challenge_bit_len;
        if sub_arity != 0 {
            base_challenge_bit_len -= sub_arity.trailing_zeros() as usize;
        }
        if top_arity != 0 {
            base_challenge_bit_len -= top_arity.trailing_zeros() as usize;
        }
        base_challenge_bit_len / (base_arity.trailing_zeros() as usize)
    };

    let mut path = vec![vec![Value::unknown(); base_arity - 1]; base_height];
    if sub_arity != 0 {
        path.push(vec![Value::unknown(); sub_arity - 1]);
    }
    if top_arity != 0 {
        path.push(vec![Value::unknown(); top_arity - 1]);
    }

    path
}

#[inline]
pub fn change_hasher_insert_arity<H, A1, A2>(
    hasher_config: <H as Halo2Hasher<A1>>::Config,
    insert_config: InsertConfig<H::Field, A1>,
) -> (<H as Halo2Hasher<A2>>::Config, InsertConfig<H::Field, A2>)
where
    H::Field: FieldExt,
    H: Halo2Hasher<A1> + Halo2Hasher<A2>,
    A1: PoseidonArity<H::Field>,
    A2: PoseidonArity<H::Field>,
{
    (
        <H as Halo2Hasher<A1>>::change_config_arity::<A2>(hasher_config),
        insert_config.change_arity::<A2>(),
    )
}

pub mod test_circuit {
    use super::*;

    use std::any::TypeId;
    use std::convert::TryInto;
    use std::marker::PhantomData;

    use fil_halo2_gadgets::{
        uint32::{UInt32Chip, UInt32Config},
        ColumnBuilder,
    };
    use filecoin_hashers::{poseidon::PoseidonHasher, sha256::Sha256Hasher};
    use halo2_proofs::{
        circuit::SimpleFloorPlanner,
        plonk::{Circuit, Column, ConstraintSystem, Instance},
    };

    use crate::halo2::{CircuitRows, Halo2Field};

    #[derive(Clone)]
    pub struct MerkleCircuitConfig<H, U, V, W, const LEAFS: usize>
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: Halo2Field,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        pub uint32: UInt32Config<H::Field>,
        pub base_hasher: <H as Halo2Hasher<U>>::Config,
        pub base_insert: InsertConfig<H::Field, U>,
        pub sub: Option<(<H as Halo2Hasher<V>>::Config, InsertConfig<H::Field, V>)>,
        pub top: Option<(<H as Halo2Hasher<W>>::Config, InsertConfig<H::Field, W>)>,
        pub pi: Column<Instance>,
    }

    #[derive(Clone)]
    pub struct MerkleCircuit<H, U, V, W, const LEAFS: usize>
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: Halo2Field,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        pub leaf: Value<H::Field>,
        pub path: Vec<Vec<Value<H::Field>>>,
        pub _tree: std::marker::PhantomData<(H, U, V, W)>,
    }

    impl<H, U, V, W, const LEAFS: usize> Circuit<H::Field> for MerkleCircuit<H, U, V, W, LEAFS>
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: Halo2Field,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        type Config = MerkleCircuitConfig<H, U, V, W, LEAFS>;
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            MerkleCircuit {
                leaf: Value::unknown(),
                path: empty_path::<H::Field, U, V, W, LEAFS>(),
                _tree: PhantomData,
            }
        }

        #[allow(clippy::unwrap_used)]
        fn configure(meta: &mut ConstraintSystem<H::Field>) -> Self::Config {
            let (advice_eq, advice_neq, fixed_eq, fixed_neq) = ColumnBuilder::new()
                .with_chip::<UInt32Chip<H::Field>>()
                .with_chip::<<H as Halo2Hasher<U>>::Chip>()
                .with_chip::<<H as Halo2Hasher<V>>::Chip>()
                .with_chip::<<H as Halo2Hasher<W>>::Chip>()
                .with_chip::<InsertChip<H::Field, U>>()
                .create_columns(meta);

            let uint32 = UInt32Chip::configure(meta, advice_eq[..9].try_into().unwrap());
            let base_hasher = <H as Halo2Hasher<U>>::configure(
                meta,
                &advice_eq,
                &advice_neq,
                &fixed_eq,
                &fixed_neq,
            );
            let base_insert = InsertChip::configure(meta, &advice_eq, &advice_neq);

            let base_arity = U::to_usize();
            let sub_arity = V::to_usize();
            let top_arity = W::to_usize();

            let sub = if sub_arity == 0 {
                None
            } else if sub_arity == base_arity {
                Some(change_hasher_insert_arity::<H, U, V>(
                    base_hasher.clone(),
                    base_insert.clone(),
                ))
            } else {
                let sub_hasher = <H as Halo2Hasher<V>>::configure(
                    meta,
                    &advice_eq,
                    &advice_neq,
                    &fixed_eq,
                    &fixed_neq,
                );
                let sub_insert = InsertChip::configure(meta, &advice_eq, &advice_neq);
                Some((sub_hasher, sub_insert))
            };

            let top = if top_arity == 0 {
                None
            } else if top_arity == base_arity {
                Some(change_hasher_insert_arity::<H, U, W>(
                    base_hasher.clone(),
                    base_insert.clone(),
                ))
            } else if top_arity == sub_arity {
                let (sub_hasher, sub_insert) = sub.clone().unwrap();
                Some(change_hasher_insert_arity::<H, V, W>(
                    sub_hasher, sub_insert,
                ))
            } else {
                let top_hasher = <H as Halo2Hasher<W>>::configure(
                    meta,
                    &advice_eq,
                    &advice_neq,
                    &fixed_eq,
                    &fixed_neq,
                );
                let top_insert = InsertChip::configure(meta, &advice_eq, &advice_neq);
                Some((top_hasher, top_insert))
            };

            let pi = meta.instance_column();
            meta.enable_equality(pi);

            MerkleCircuitConfig {
                uint32,
                base_hasher,
                base_insert,
                sub,
                top,
                pi,
            }
        }

        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<H::Field>,
        ) -> Result<(), Error> {
            // Absolute rows of public inputs.
            const CHALLENGE_ROW: usize = 0;
            const ROOT_ROW: usize = 1;

            let MerkleCircuitConfig {
                uint32: uint32_config,
                base_hasher: base_hasher_config,
                base_insert: base_insert_config,
                sub: sub_config,
                top: top_config,
                pi: pi_col,
            } = config;

            let uint32_chip = UInt32Chip::construct(uint32_config);

            <H as Halo2Hasher<U>>::load(&mut layouter, &base_hasher_config)?;
            let base_hasher_chip = <H as Halo2Hasher<U>>::construct(base_hasher_config);
            let base_insert_chip = InsertChip::construct(base_insert_config);

            let sub_chips = sub_config.map(|(hasher_config, insert_config)| {
                let hasher_chip = <H as Halo2Hasher<V>>::construct(hasher_config);
                let insert_chip = InsertChip::construct(insert_config);
                (hasher_chip, insert_chip)
            });

            let top_chips = top_config.map(|(hasher_config, insert_config)| {
                let hasher_chip = <H as Halo2Hasher<W>>::construct(hasher_config);
                let insert_chip = InsertChip::construct(insert_config);
                (hasher_chip, insert_chip)
            });

            let merkle_chip = MerkleChip::<H, U, V, W>::with_subchips(
                base_hasher_chip,
                base_insert_chip,
                sub_chips,
                top_chips,
            );

            let challenge_bits = uint32_chip.pi_assign_bits(
                layouter.namespace(|| "assign challenge pi as 32 bits"),
                pi_col,
                CHALLENGE_ROW,
            )?;

            let root = merkle_chip.compute_root(
                layouter.namespace(|| "compute merkle root"),
                &challenge_bits,
                self.leaf,
                &self.path,
            )?;
            layouter.constrain_instance(root.cell(), pi_col, ROOT_ROW)
        }
    }

    impl<H, U, V, W, const LEAFS: usize> CircuitRows for MerkleCircuit<H, U, V, W, LEAFS>
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: Halo2Field,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        fn k(&self) -> u32 {
            let hasher_type = TypeId::of::<H>();
            if hasher_type == TypeId::of::<Sha256Hasher<H::Field>>() {
                // TODO (jake): under which arities and tree size does this increase?
                17
            } else if hasher_type == TypeId::of::<PoseidonHasher<H::Field>>() {
                use neptune::halo2_circuit::PoseidonChip;

                let base_arity = U::to_usize();
                let sub_arity = V::to_usize();
                let top_arity = W::to_usize();

                let base_bit_len = base_arity.trailing_zeros() as usize;
                let sub_bit_len = sub_arity.trailing_zeros() as usize;
                let top_bit_len = top_arity.trailing_zeros() as usize;

                let mut base_challenge_bit_len = LEAFS.trailing_zeros() as usize;
                if sub_arity > 0 {
                    base_challenge_bit_len -= sub_bit_len;
                }
                if top_arity > 0 {
                    base_challenge_bit_len -= top_bit_len;
                }
                let base_path_len = base_challenge_bit_len / base_bit_len;

                // Four rows for decomposing the challenge into 32 bits.
                let challenge_decomp_rows = 4;
                let base_hasher_rows = PoseidonChip::<H::Field, U>::num_rows();
                let sub_hasher_rows = PoseidonChip::<H::Field, V>::num_rows();
                let top_hasher_rows = PoseidonChip::<H::Field, W>::num_rows();
                // One row per insert.
                let insert_rows = 1;

                let mut rows = challenge_decomp_rows;
                rows += base_path_len * (base_hasher_rows + insert_rows);
                if sub_arity > 0 {
                    rows += sub_hasher_rows + insert_rows;
                }
                if top_arity > 0 {
                    rows += top_hasher_rows + insert_rows;
                };

                (rows as f32).log2().ceil() as u32
            } else {
                unimplemented!("hasher must be poseidon or sha256");
            }
        }
    }

    impl<H, U, V, W, const LEAFS: usize> MerkleCircuit<H, U, V, W, LEAFS>
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: Halo2Field,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        pub fn new(leaf: H::Field, path: Vec<Vec<H::Field>>) -> Self {
            MerkleCircuit {
                leaf: Value::known(leaf),
                path: path
                    .iter()
                    .map(|sibs| sibs.iter().copied().map(Value::known).collect())
                    .collect(),
                _tree: PhantomData,
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    use std::any::TypeId;
    use std::convert::TryInto;
    use std::marker::PhantomData;

    use ff::PrimeField;
    use fil_halo2_gadgets::{
        uint32::{AssignedU32, UInt32Chip, UInt32Config},
        ColumnBuilder,
    };
    use filecoin_hashers::{poseidon::PoseidonHasher, sha256::Sha256Hasher, HashFunction};
    use generic_array::typenum::{U0, U2, U4, U8};
    use halo2_proofs::{
        circuit::SimpleFloorPlanner,
        dev::MockProver,
        pasta::Fp,
        plonk::{Circuit, ConstraintSystem},
    };
    use merkletree::store::VecStore;
    use rand::{Rng, SeedableRng};
    use rand_xorshift::XorShiftRng;

    use crate::{
        halo2::gadgets::insert::InsertConfig,
        merkle::{
            generate_tree, MerkleProof, MerkleProofTrait, MerkleTreeTrait, MerkleTreeWrapper,
        },
        util::NODE_SIZE,
        TEST_SEED,
    };

    #[derive(Clone)]
    struct MyConfig<H, U, V, W>
    where
        H: Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: FieldExt,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        uint32: UInt32Config<H::Field>,
        base_hasher: <H as Halo2Hasher<U>>::Config,
        base_insert: InsertConfig<H::Field, U>,
        sub: Option<(<H as Halo2Hasher<V>>::Config, InsertConfig<H::Field, V>)>,
        top: Option<(<H as Halo2Hasher<W>>::Config, InsertConfig<H::Field, W>)>,
    }

    struct MyCircuit<H, U, V, W>
    where
        H: Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: FieldExt,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        challenge: Value<u32>,
        leaf: Value<H::Field>,
        path: Vec<Vec<Value<H::Field>>>,
        _u: PhantomData<U>,
        _v: PhantomData<V>,
        _w: PhantomData<W>,
    }

    impl<H, U, V, W> Circuit<H::Field> for MyCircuit<H, U, V, W>
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: FieldExt,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        type Config = MyConfig<H, U, V, W>;
        type FloorPlanner = SimpleFloorPlanner;

        fn without_witnesses(&self) -> Self {
            MyCircuit {
                challenge: Value::unknown(),
                leaf: Value::unknown(),
                path: self
                    .path
                    .iter()
                    .map(|sibs| vec![Value::unknown(); sibs.len()])
                    .collect(),
                _u: PhantomData,
                _v: PhantomData,
                _w: PhantomData,
            }
        }

        #[allow(clippy::unwrap_used)]
        fn configure(meta: &mut ConstraintSystem<H::Field>) -> Self::Config {
            let base_arity = U::to_usize();
            let sub_arity = V::to_usize();
            let top_arity = W::to_usize();

            let (advice_eq, advice_neq, fixed_eq, fixed_neq) = ColumnBuilder::new()
                .with_chip::<UInt32Chip<H::Field>>()
                .with_chip::<<H as Halo2Hasher<U>>::Chip>()
                .with_chip::<<H as Halo2Hasher<V>>::Chip>()
                .with_chip::<<H as Halo2Hasher<W>>::Chip>()
                // Only need the base arity here because it is guaranteed to be the largest arity,
                // thus all other arity insert chips will use a subset of the base arity's columns.
                .with_chip::<InsertChip<H::Field, U>>()
                .create_columns(meta);

            let uint32 = UInt32Chip::configure(meta, advice_eq[..9].try_into().unwrap());

            let base_hasher = <H as Halo2Hasher<U>>::configure(
                meta,
                &advice_eq,
                &advice_neq,
                &fixed_eq,
                &fixed_neq,
            );
            let base_insert = InsertChip::configure(meta, &advice_eq, &advice_neq);

            let sub = if sub_arity == 0 {
                None
            } else if sub_arity == base_arity {
                Some(change_hasher_insert_arity::<H, U, V>(
                    base_hasher.clone(),
                    base_insert.clone(),
                ))
            } else {
                let sub_hasher = <H as Halo2Hasher<V>>::configure(
                    meta,
                    &advice_eq,
                    &advice_neq,
                    &fixed_eq,
                    &fixed_neq,
                );
                let sub_insert = InsertChip::configure(meta, &advice_eq, &advice_neq);
                Some((sub_hasher, sub_insert))
            };

            let top = if top_arity == 0 {
                None
            } else if top_arity == base_arity {
                Some(change_hasher_insert_arity::<H, U, W>(
                    base_hasher.clone(),
                    base_insert.clone(),
                ))
            } else if top_arity == sub_arity {
                let (sub_hasher, sub_insert) = sub.clone().unwrap();
                Some(change_hasher_insert_arity::<H, V, W>(
                    sub_hasher, sub_insert,
                ))
            } else {
                let top_hasher = <H as Halo2Hasher<W>>::configure(
                    meta,
                    &advice_eq,
                    &advice_neq,
                    &fixed_eq,
                    &fixed_neq,
                );
                let top_insert = InsertChip::configure(meta, &advice_eq, &advice_neq);
                Some((top_hasher, top_insert))
            };

            MyConfig {
                uint32,
                base_hasher,
                base_insert,
                sub,
                top,
            }
        }

        #[allow(clippy::unwrap_used)]
        fn synthesize(
            &self,
            config: Self::Config,
            mut layouter: impl Layouter<H::Field>,
        ) -> Result<(), Error> {
            let MyConfig {
                uint32: uint32_config,
                base_hasher: base_hasher_config,
                base_insert: base_insert_config,
                sub: sub_config,
                top: top_config,
            } = config;

            let challenge_col = uint32_config.value_col();

            let uint32_chip = UInt32Chip::construct(uint32_config);

            <H as Halo2Hasher<U>>::load(&mut layouter, &base_hasher_config)?;
            let base_hasher_chip = <H as Halo2Hasher<U>>::construct(base_hasher_config);
            let base_insert_chip = InsertChip::construct(base_insert_config);

            let sub_hasher_insert_chips = sub_config.map(|(hasher_config, insert_config)| {
                let hasher_chip = <H as Halo2Hasher<V>>::construct(hasher_config);
                let insert_chip = InsertChip::construct(insert_config);
                (hasher_chip, insert_chip)
            });

            let top_hasher_insert_chips = top_config.map(|(hasher_config, insert_config)| {
                let hasher_chip = <H as Halo2Hasher<W>>::construct(hasher_config);
                let insert_chip = InsertChip::construct(insert_config);
                (hasher_chip, insert_chip)
            });

            let merkle_chip = MerkleChip::<H, U, V, W>::with_subchips(
                base_hasher_chip,
                base_insert_chip,
                sub_hasher_insert_chips,
                top_hasher_insert_chips,
            );

            let challenge_bits = layouter.assign_region(
                || "challenge",
                |mut region| {
                    let offset = 0;
                    let challenge = AssignedU32::assign(
                        &mut region,
                        || "challenge",
                        challenge_col,
                        offset,
                        self.challenge,
                    )?;
                    uint32_chip.assign_bits(&mut region, offset, challenge)
                },
            )?;

            merkle_chip
                .compute_root(layouter, &challenge_bits, self.leaf, &self.path)?
                .value()
                .zip(Value::known(self.expected_root()).as_ref())
                .assert_if_known(|(root, expected_root)| root == expected_root);

            Ok(())
        }
    }

    impl<H, U, V, W> MyCircuit<H, U, V, W>
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: FieldExt,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        fn with_witness(challenge: u32, merkle_proof: &MerkleProof<H, U, V, W>) -> Self {
            let path = merkle_proof
                .path()
                .iter()
                .map(|(sibs, _)| {
                    sibs.iter()
                        .copied()
                        .map(|sib| Value::known(sib.into()))
                        .collect()
                })
                .collect();
            MyCircuit {
                challenge: Value::known(challenge),
                leaf: Value::known(merkle_proof.leaf().into()),
                path,
                _u: PhantomData,
                _v: PhantomData,
                _w: PhantomData,
            }
        }

        fn k(num_leafs: usize) -> u32 {
            let hasher_type = TypeId::of::<H>();
            if hasher_type == TypeId::of::<Sha256Hasher<H::Field>>() {
                return 17;
            }
            assert_eq!(hasher_type, TypeId::of::<PoseidonHasher<H::Field>>());

            let challenge_bit_len = num_leafs.trailing_zeros() as usize;

            let base_arity = U::to_usize();
            let sub_arity = V::to_usize();
            let top_arity = W::to_usize();

            let base_bit_len = base_arity.trailing_zeros() as usize;
            let sub_bit_len = sub_arity.trailing_zeros() as usize;
            let top_bit_len = top_arity.trailing_zeros() as usize;

            let base_path_len = if top_arity > 0 {
                (challenge_bit_len - (top_bit_len + sub_bit_len)) / base_bit_len
            } else if sub_arity > 0 {
                (challenge_bit_len - sub_bit_len) / base_bit_len
            } else {
                challenge_bit_len / base_bit_len
            };

            use neptune::halo2_circuit::PoseidonChip;
            let base_hasher_rows = PoseidonChip::<H::Field, U>::num_rows();
            let sub_hasher_rows = PoseidonChip::<H::Field, V>::num_rows();
            let top_hasher_rows = PoseidonChip::<H::Field, W>::num_rows();
            let insert_rows = 1;

            // Four rows for decomposing the challenge into 32 bits.
            let mut num_rows = 4;
            num_rows += base_path_len * (base_hasher_rows + insert_rows);
            if sub_arity > 0 {
                num_rows += sub_hasher_rows + insert_rows;
            }
            if top_arity > 0 {
                num_rows += top_hasher_rows + insert_rows;
            };

            (num_rows as f32).log2().ceil() as u32
        }

        #[allow(clippy::unwrap_used)]
        fn expected_root(&self) -> H::Field {
            let challenge = self.challenge.map(|c| c as usize);

            let mut challenge_bits = vec![0; 32];
            challenge.map(|challenge| {
                for (i, bit) in challenge_bits.iter_mut().enumerate() {
                    *bit = challenge >> i & 1;
                }
            });
            let mut challenge_bits = challenge_bits.into_iter();

            let mut cur = H::Field::default();
            self.leaf.map(|leaf| {
                cur = leaf;
            });

            for siblings in self.path.iter() {
                let arity = siblings.len() + 1;
                let index_bit_len = arity.trailing_zeros() as usize;

                let mut index = 0;
                for i in 0..index_bit_len {
                    index += challenge_bits.next().unwrap() << i;
                }

                // Insert `cur` into `siblings` at position `index`.
                let mut preimage = Vec::<u8>::with_capacity(arity * NODE_SIZE);
                for sib in &siblings[..index] {
                    sib.map(|sib| {
                        preimage.extend_from_slice(sib.to_repr().as_ref());
                    });
                }
                preimage.extend_from_slice(cur.to_repr().as_ref());
                for sib in &siblings[index..] {
                    sib.map(|sib| {
                        preimage.extend_from_slice(sib.to_repr().as_ref());
                    });
                }

                cur = H::Function::hash(&preimage).into();
            }

            cur
        }
    }

    #[allow(clippy::unwrap_used)]
    fn test_merkle_chip_inner<H, U, V, W>()
    where
        H: 'static + Halo2Hasher<U> + Halo2Hasher<V> + Halo2Hasher<W>,
        H::Field: FieldExt,
        U: PoseidonArity<H::Field>,
        V: PoseidonArity<H::Field>,
        W: PoseidonArity<H::Field>,
    {
        const BASE_HEIGHT: u32 = 2;

        let base_arity = U::to_usize();
        let sub_arity = V::to_usize();
        let top_arity = W::to_usize();

        let mut num_leafs = base_arity.pow(BASE_HEIGHT);
        if sub_arity > 0 {
            num_leafs *= sub_arity;
        }
        if top_arity > 0 {
            num_leafs *= top_arity;
        }

        let mut rng = XorShiftRng::from_seed(TEST_SEED);
        let (_, tree) = generate_tree::<MerkleTreeWrapper<H, VecStore<H::Domain>, U, V, W>, _>(
            &mut rng, num_leafs, None,
        );

        let challenges: Vec<usize> = if num_leafs < 10 {
            (0..num_leafs).collect()
        } else {
            (0..10).map(|_| rng.gen::<usize>() % num_leafs).collect()
        };

        for challenge in challenges {
            let merkle_proof = tree.gen_proof(challenge).unwrap();
            let circ = MyCircuit::<H, U, V, W>::with_witness(challenge as u32, &merkle_proof);
            let k = MyCircuit::<H, U, V, W>::k(num_leafs);
            let prover = MockProver::run(k, &circ, vec![]).unwrap();
            assert!(prover.verify().is_ok());
        }
    }

    #[test]
    fn test_merkle_chip_poseidon_2() {
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U2, U0, U0>();
    }

    #[test]
    fn test_merkle_chip_poseidon_4() {
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U4, U0, U0>();
    }

    #[test]
    fn test_merkle_chip_poseidon_8() {
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U8, U0, U0>();
    }

    #[test]
    fn test_merkle_chip_poseidon_8_2() {
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U8, U2, U0>();
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U8, U8, U2>();
    }

    #[test]
    fn test_merkle_chip_poseidon_8_4() {
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U8, U4, U0>();
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U8, U8, U4>();
    }

    #[test]
    fn test_merkle_chip_poseidon_8_4_2() {
        test_merkle_chip_inner::<PoseidonHasher<Fp>, U8, U4, U2>();
    }

    #[test]
    fn test_merkle_chip_sha256_2() {
        test_merkle_chip_inner::<Sha256Hasher<Fp>, U2, U0, U0>();
    }
}