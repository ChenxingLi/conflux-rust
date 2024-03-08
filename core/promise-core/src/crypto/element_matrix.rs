use std::{
    collections::{BTreeMap, BTreeSet},
    ops::{Index, IndexMut},
};

use crate::{cfg_into_iter, cfg_iter_mut, converted_id::num_to_identifier};

use super::{
    interpolate_and_evaluate_points,
    types::{Element, PolynomialCommitment as PC, Scalar},
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

#[derive(Clone)]
pub struct Matrix<T: Clone> {
    rows: usize,
    cols: usize,
    inner: Vec<T>,
}

pub type ElementMatrix = Matrix<Element>;
#[allow(dead_code)]
pub type ScalarMatrix = Matrix<Scalar>;

impl<T: Clone> Matrix<T> {
    pub fn new(rows: usize, cols: usize) -> Self
    where T: Default {
        let zero = Default::default();
        Self {
            rows,
            cols,
            inner: vec![zero; rows * cols],
        }
    }

    pub fn size(&self) -> (usize, usize) { (self.cols, self.rows) }

    pub fn cols(&self) -> usize { self.cols }

    pub fn rows(&self) -> usize { self.rows }

    pub fn create_empty(&self) -> Self
    where T: Default {
        Self::new(self.rows, self.cols)
    }

    pub fn set_row(&mut self, row_idx: usize, items: &[T]) {
        assert!(row_idx < self.rows);
        assert!(items.len() == self.cols);

        for (col_idx, item) in items.iter().enumerate() {
            self[(col_idx, row_idx)] = item.clone();
        }
    }

    pub fn set_col(&mut self, col_idx: usize, items: &[T]) {
        assert!(col_idx < self.cols);
        assert!(items.len() == self.rows);

        let slots =
            &mut self.inner[self.rows * col_idx..self.rows * (col_idx + 1)];
        slots.clone_from_slice(&items[..]);
    }

    pub fn get_col(&self, col_idx: usize) -> Vec<T> {
        assert!(col_idx < self.cols);

        let slots = &self.inner[self.rows * col_idx..self.rows * (col_idx + 1)];
        slots.to_vec()
    }
}

impl Matrix<Element> {
    pub fn evaluate_row(&mut self, row_idx: usize, commitment: &PC) {
        let elements: Vec<Element> = cfg_into_iter!(0..self.cols)
            .map(|col_idx| {
                frost_core::keys::evaluate_vss(
                    num_to_identifier(col_idx),
                    commitment,
                )
            })
            .collect();
        self.set_row(row_idx, &elements);
    }

    pub fn interpolate_col(&mut self, col_idx: usize, filled: BTreeSet<usize>) {
        let filled_points: BTreeMap<usize, Element> = filled
            .into_iter()
            .map(|x| (x, self[(col_idx, x)].clone()))
            .collect();
        let evaluated_points =
            interpolate_and_evaluate_points(filled_points, 0..self.rows);
        for (row_idx, elem) in evaluated_points.into_iter() {
            self[(col_idx, row_idx)] = elem;
        }
    }

    pub fn get_col_add(&self, col_idx: usize, commitment: &PC) -> Vec<Element> {
        assert!(col_idx < self.cols);

        let mut elements = self.get_col(col_idx);
        cfg_iter_mut!(elements)
            .enumerate()
            .for_each(|(row_idx, element)| {
                *element += frost_core::keys::evaluate_vss(
                    num_to_identifier(row_idx),
                    commitment,
                );
            });
        elements
    }
}

impl<T: Clone> Index<(usize, usize)> for Matrix<T> {
    type Output = T;

    fn index(&self, (col_idx, row_idx): (usize, usize)) -> &Self::Output {
        &self.inner[col_idx * self.rows + row_idx]
    }
}

impl<T: Clone> IndexMut<(usize, usize)> for Matrix<T> {
    fn index_mut(
        &mut self, (col_idx, row_idx): (usize, usize),
    ) -> &mut Self::Output {
        &mut self.inner[col_idx * self.rows + row_idx]
    }
}
