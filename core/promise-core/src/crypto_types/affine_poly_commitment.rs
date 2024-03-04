use std::ops::Deref;

use super::{Affine, CoefficientCommitment, PolynomialCommitment, Projective};

use cfx_types::H256;
use group::{prime::PrimeCurveAffine, Curve, GroupEncoding};
use serde::{Deserialize, Serialize};
use tiny_keccak::{Hasher, Keccak};

#[derive(Serialize, Deserialize)]
pub struct AffinePolynomialCommitment(Vec<Affine>);

impl AffinePolynomialCommitment {
    pub fn hash(&self) -> H256 {
        let mut hasher = Keccak::v256();
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
        let mut affine_points = vec![Affine::IDENTITY; points.len()];
        Projective::batch_normalize(&points[..], &mut affine_points[..]);
        Self(affine_points)
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
