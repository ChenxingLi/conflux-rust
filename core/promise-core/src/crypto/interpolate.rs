use std::{
    collections::{BTreeMap, BTreeSet},
    iter::zip,
};

use crate::{cfg_into_iter, converted_id::num_to_identifier};

use super::types::Scalar;

use frost_core::VartimeMultiscalarMul;
use frost_secp256k1::{Identifier, Secp256K1Sha256};

use k256::elliptic_curve::ops::BatchInvert;

#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub fn interpolate_and_evaluate_share<T: GroupOp>(
    input_points: BTreeMap<usize, T>,
) -> T {
    assert!(!input_points.contains_key(&0));

    let (_, ans) =
        interpolate_and_evaluate_points(input_points, std::iter::once(0))
            .pop_first()
            .unwrap();
    ans
}
pub fn interpolate_and_evaluate_points<T: GroupOp>(
    input_points: BTreeMap<usize, T>,
    to_evaluate: impl IntoIterator<Item = usize>,
) -> BTreeMap<usize, T> {
    let to_evaluate: Vec<usize> = to_evaluate
        .into_iter()
        .filter(|x| !input_points.contains_key(x))
        .collect();

    let input_points: BTreeMap<Identifier, T> = input_points
        .into_iter()
        .map(|(x, y)| (num_to_identifier(x), y))
        .collect();

    let x_set: BTreeSet<Identifier> =
        input_points.iter().map(|(x, _)| x.clone()).collect();

    #[cfg(feature = "parallel")]
    let min_len = std::cmp::max(
        1,
        5_000_000 / (input_points.len() * T::ESTIMATE_MSM_TIME),
    );

    cfg_into_iter!(to_evaluate, min_len)
        .map(|evaluate_point| {
            let ans = interpolate_and_evaluate_points_inner(
                &x_set,
                &input_points,
                num_to_identifier(evaluate_point),
            );
            (evaluate_point, ans)
        })
        .collect()
}

fn interpolate_and_evaluate_points_inner<T: GroupOp>(
    x_set: &BTreeSet<Identifier>, input_points: &BTreeMap<Identifier, T>,
    evaluate_point: Identifier,
) -> T {
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
    for (num, inv_den) in zip(&mut num_list, inv_den_list) {
        *num *= inv_den;
    }

    GroupOp::multi_scalar_mul(elem_list, num_list)
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

pub trait GroupOp
where Self: Send + Sync + Sized + Clone
{
    const ESTIMATE_MSM_TIME: usize;
    fn multi_scalar_mul(base: Vec<Self>, scalar: Vec<Scalar>) -> Self;
}

impl GroupOp for k256::Scalar {
    const ESTIMATE_MSM_TIME: usize = 5;

    fn multi_scalar_mul(base: Vec<Self>, scalar: Vec<Scalar>) -> Self {
        assert_eq!(base.len(), scalar.len());

        zip(base, scalar).map(|(x, y)| x * y).sum()
    }
}

impl GroupOp for k256::ProjectivePoint {
    const ESTIMATE_MSM_TIME: usize = 2000;

    fn multi_scalar_mul(base: Vec<Self>, scalar: Vec<Scalar>) -> Self {
        assert_eq!(base.len(), scalar.len());

        // zip(base, scalar).map(|(x, y)| x * y).sum()
        VartimeMultiscalarMul::<Secp256K1Sha256>::vartime_multiscalar_mul(
            scalar, base,
        )
    }
}
