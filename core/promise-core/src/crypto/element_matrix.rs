use std::{
    collections::{BTreeMap, BTreeSet},
    ops::{Index, IndexMut},
};

use crate::{cfg_into_iter, cfg_iter_mut, converted_id::num_to_identifier};

use super::{
    interpolate_and_evaluate_points,
    types::{Element, PolynomialCommitment as PC},
};

#[cfg(feature = "parallel")]
use rayon::prelude::*;

pub struct ElementMatrix {
    rows: usize,
    cols: usize,
    inner: Vec<Element>,
}

impl ElementMatrix {
    pub fn new(rows: usize, cols: usize) -> Self {
        let zero = Element::IDENTITY;
        Self {
            rows,
            cols,
            inner: vec![zero; rows * cols],
        }
    }

    pub fn size(&self) -> (usize, usize) { (self.cols, self.rows) }

    pub fn create_empty(&self) -> Self { Self::new(self.rows, self.cols) }

    pub fn set_row(&mut self, row_idx: usize, elements: &[Element]) {
        assert!(row_idx < self.rows);
        assert!(elements.len() == self.cols);

        for (col_idx, elem) in elements.iter().enumerate() {
            self[(col_idx, row_idx)] = elem.clone();
        }
    }

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

    pub fn set_col(&mut self, col_idx: usize, elements: &[Element]) {
        assert!(col_idx < self.cols);
        assert!(elements.len() == self.rows);

        let slots =
            &mut self.inner[self.rows * col_idx..self.rows * (col_idx + 1)];
        slots.clone_from_slice(&elements[..]);
    }

    pub fn get_col(&self, col_idx: usize) -> Vec<Element> {
        assert!(col_idx < self.cols);

        let slots = &self.inner[self.rows * col_idx..self.rows * (col_idx + 1)];
        slots.to_vec()
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
}

impl Index<(usize, usize)> for ElementMatrix {
    type Output = Element;

    fn index(&self, (col_idx, row_idx): (usize, usize)) -> &Self::Output {
        &self.inner[col_idx * self.rows + row_idx]
    }
}

impl IndexMut<(usize, usize)> for ElementMatrix {
    fn index_mut(
        &mut self, (col_idx, row_idx): (usize, usize),
    ) -> &mut Self::Output {
        &mut self.inner[col_idx * self.rows + row_idx]
    }
}
