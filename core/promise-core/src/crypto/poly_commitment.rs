use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Deref,
};

use crate::{cfg_into_iter, cfg_iter, converted_id::VoteID};

use super::types::{
    Affine, CoefficientCommitment, Element, PolynomialCommitment, Projective,
};

use cfx_types::H256;
use frost_core::VartimeMultiscalarMul;
use frost_secp256k1::{Identifier, Secp256K1Sha256};
use group::{prime::PrimeCurveAffine, GroupEncoding};
use k256::{
    elliptic_curve::{
        ops::{BatchInvert, MulByGenerator},
        BatchNormalize,
    },
    Scalar,
};
use rand_core::{CryptoRng, RngCore};
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[derive(Serialize, Deserialize, Clone)]
pub struct AffinePolynomialCommitment(Vec<Affine>);

impl AffinePolynomialCommitment {
    pub fn hash(&self) -> H256 {
        let mut hasher = Keccak::v256();
        hasher.update(b"cfx-promise-polynomial-commitment-secp256k1");
        for point in &self.0 {
            hasher.update(point.to_bytes().as_slice());
        }
        let mut digest = H256::zero();
        hasher.finalize(&mut digest.0);
        digest
    }
}

impl From<PolynomialCommitment> for AffinePolynomialCommitment {
    fn from(commitment: PolynomialCommitment) -> Self {
        let points: Vec<Projective> = commitment
            .coefficients()
            .iter()
            .map(|x| x.value())
            .collect();
        Self(Projective::batch_normalize(&points[..]))
    }
}

impl From<AffinePolynomialCommitment> for PolynomialCommitment {
    fn from(value: AffinePolynomialCommitment) -> Self {
        let mut x = vec![];
        for point in value.0 {
            x.push(CoefficientCommitment::new(point.to_curve()));
        }
        Self::new(x)
    }
}

impl Deref for AffinePolynomialCommitment {
    type Target = Vec<Affine>;

    fn deref(&self) -> &Self::Target { &self.0 }
}

pub fn add_commitment(
    a: &PolynomialCommitment, b: &PolynomialCommitment,
) -> PolynomialCommitment {
    let a_coeff = a.coefficients();
    let b_coeff = b.coefficients();
    let length = std::cmp::max(a.coefficients().len(), b.coefficients().len());

    let mut answer = vec![];
    for i in 0..length {
        let a_i = a_coeff.get(i).map_or(Element::IDENTITY, |x| x.value());
        let b_i = b_coeff.get(i).map_or(Element::IDENTITY, |x| x.value());
        let c_i = CoefficientCommitment::new(a_i + b_i);
        answer.push(c_i)
    }
    PolynomialCommitment::new(answer)
}

pub fn generate_polynomial_commitments<R: CryptoRng + RngCore>(
    rng: &mut R, degree: usize, vote_id: Vec<VoteID>,
    shared_input: Option<Scalar>,
) -> (PolynomialCommitment, BTreeMap<VoteID, (Scalar, Element)>) {
    let constant_coefficient = if let Some(specified) = shared_input {
        specified
    } else {
        Scalar::generate_biased(rng)
    };
    let mut scalar_coefficients = vec![constant_coefficient];
    for _ in 0..degree {
        scalar_coefficients.push(Scalar::generate_biased(rng));
    }

    let shares: BTreeMap<VoteID, (Scalar, Element)> = cfg_iter!(vote_id, 50)
        .map(|id| {
            let share = frost_core::keys::evaluate_polynomial(
                id.to_identifier(),
                &scalar_coefficients,
            );
            let share_commit = Element::mul_by_generator(&share);
            (*id, (share, share_commit))
        })
        .collect();

    let element_coefficients: Vec<CoefficientCommitment> =
        cfg_iter!(scalar_coefficients, 50)
            .map(|scalar| {
                CoefficientCommitment::new(Element::mul_by_generator(scalar))
            })
            .collect();
    let polynomial_commitment = PolynomialCommitment::new(element_coefficients);

    return (polynomial_commitment, shares);
}

pub fn validate_verifiable_secret_share(
    commitment: &PolynomialCommitment, shares: &BTreeMap<VoteID, Scalar>,
) -> bool {
    let diff_list: Vec<Projective> = cfg_iter!(shares)
        .map(|(vote_id, share)| {
            let expected = frost_core::keys::evaluate_vss(
                vote_id.to_identifier(),
                commitment,
            );
            let actual = Projective::mul_by_generator(share);
            expected - actual
        })
        .collect();

    let diff_list = Projective::batch_normalize(&diff_list[..]);

    for diff in diff_list.into_iter() {
        if (!diff.is_identity()).into() {
            return false;
        }
    }

    true
}

pub fn interpolate_and_evaluate_points(
    input_points: BTreeMap<usize, Element>,
    to_evaluate: impl IntoIterator<Item = usize>,
) -> BTreeMap<usize, Element> {
    let to_evaluate: Vec<usize> = to_evaluate
        .into_iter()
        .filter(|x| !input_points.contains_key(x))
        .collect();

    let input_points: BTreeMap<Identifier, Element> = input_points
        .into_iter()
        .map(|(x, y)| (cell_to_identifier(x), y))
        .collect();

    let x_set: BTreeSet<Identifier> =
        input_points.iter().map(|(x, _)| x.clone()).collect();

    cfg_into_iter!(to_evaluate)
        .map(|evaluate_point| {
            let ans = interpolate_and_evaluate_points_inner(
                &x_set,
                &input_points,
                cell_to_identifier(evaluate_point),
            );
            (evaluate_point, ans)
        })
        .collect()
}

fn interpolate_and_evaluate_points_inner(
    x_set: &BTreeSet<Identifier>, input_points: &BTreeMap<Identifier, Element>,
    evaluate_point: Identifier,
) -> Element {
    let mut num_list = vec![];
    let mut den_list = vec![];
    let mut elem_list = vec![];
    for (identifier, element) in input_points {
        let (num, den) = compute_lagrange_coefficient_partial(
            x_set,
            Some(evaluate_point),
            identifier.clone(),
        );
        num_list.push(num);
        den_list.push(den);
        elem_list.push(element.clone());
    }

    let inv_den_list = Scalar::batch_invert(&den_list[..]).unwrap();
    num_list
        .iter_mut()
        .zip(inv_den_list.iter())
        .for_each(|(num, inv_den_list)| *num *= *inv_den_list);

    VartimeMultiscalarMul::<Secp256K1Sha256>::vartime_multiscalar_mul(
        num_list, elem_list,
    )
}

fn compute_lagrange_coefficient_partial(
    x_set: &BTreeSet<Identifier>, x: Option<Identifier>, x_i: Identifier,
) -> (Scalar, Scalar) {
    assert!(!x_set.is_empty());

    let mut num = Scalar::ONE;
    let mut den = Scalar::ONE;

    let mut x_i_found = false;

    for x_j in x_set.iter() {
        if x_i == *x_j {
            x_i_found = true;
            continue;
        }

        if let Some(x) = x {
            num *= x - *x_j;
            den *= x_i - *x_j;
        } else {
            // Both signs inverted just to avoid requiring Neg (-*xj)
            num *= *x_j;
            den *= *x_j - x_i;
        }
    }

    assert!(!x_i_found);

    (num, den)
}

fn cell_to_identifier(x: usize) -> Identifier {
    Identifier::new(Scalar::from(x as u32 + 1)).unwrap()
}
