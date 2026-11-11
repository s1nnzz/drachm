//! Typed indexes and a `Vec` that is indexed by them.

use std::marker::PhantomData;
use std::ops::{Index, IndexMut};

/// A `u32` newtype usable as an `IndexVec` index.
pub trait Idx: Copy {
    fn new(i: usize) -> Self;
    fn index(self) -> usize;
}

macro_rules! newtype_index {
    ($(#[$m:meta])* $name:ident) => {
        $(#[$m])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub struct $name(pub u32);

        impl $crate::mir::index::Idx for $name {
            fn new(i: usize) -> Self {
                $name(u32::try_from(i).expect("index fits in u32"))
            }
            fn index(self) -> usize {
                self.0 as usize
            }
        }
    };
}
pub(crate) use newtype_index;

#[derive(Debug, Clone)]
pub struct IndexVec<I, T> {
    raw: Vec<T>,
    _marker: PhantomData<fn(&I)>,
}

impl<I: Idx, T> IndexVec<I, T> {
    pub fn new() -> Self {
        IndexVec { raw: Vec::new(), _marker: PhantomData }
    }

    pub fn from_vec(raw: Vec<T>) -> Self {
        IndexVec { raw, _marker: PhantomData }
    }

    pub fn push(&mut self, value: T) -> I {
        self.raw.push(value);
        I::new(self.raw.len() - 1)
    }

    pub fn len(&self) -> usize {
        self.raw.len()
    }

    pub fn is_empty(&self) -> bool {
        self.raw.is_empty()
    }

    pub fn get(&self, i: I) -> Option<&T> {
        self.raw.get(i.index())
    }

    pub fn iter(&self) -> std::slice::Iter<'_, T> {
        self.raw.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, T> {
        self.raw.iter_mut()
    }

    pub fn iter_enumerated(&self) -> impl Iterator<Item = (I, &T)> {
        self.raw.iter().enumerate().map(|(i, t)| (I::new(i), t))
    }

    pub fn indices(&self) -> impl Iterator<Item = I> + use<I, T> {
        (0..self.raw.len()).map(I::new)
    }

    pub fn into_vec(self) -> Vec<T> {
        self.raw
    }
}

impl<I: Idx, T> Default for IndexVec<I, T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<I: Idx, T> Index<I> for IndexVec<I, T> {
    type Output = T;
    fn index(&self, i: I) -> &T {
        &self.raw[i.index()]
    }
}

impl<I: Idx, T> IndexMut<I> for IndexVec<I, T> {
    fn index_mut(&mut self, i: I) -> &mut T {
        &mut self.raw[i.index()]
    }
}
