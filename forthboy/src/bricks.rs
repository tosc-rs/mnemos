use std::{marker::PhantomData, ptr::NonNull};

use crate::fancy::{rot_right, rot_left};

#[derive(Debug, PartialEq)]
pub struct Bricks<const L: usize> {
    idx_buf: [usize; L],
    user_editable_end: usize, //  0..ue
    inco_editable_end: usize, // ue..ie
    history_end: usize,       // ie..hi
                              // hi..   => free
}

pub struct BrickIter<'a, const L: usize, I>
{
    bricks: &'a [usize],
    collection: &'a [I],
}

pub struct BrickIterMut<'a, 'b, const L: usize, I>
{
    bricks: &'a [usize],
    col_ptr: NonNull<[I]>,
    _cpd: PhantomData<&'b mut [I]>,
}

impl<'a, const L: usize, I> Iterator for BrickIter<'a, L, I>
{
    type Item = &'a I;

    fn next(&mut self) -> Option<Self::Item> {
        let (now, remain) = self.bricks.split_first()?;
        self.bricks = remain;
        self.collection.get(*now)
    }
}

impl<'a, 'b, const L: usize, I> Iterator for BrickIterMut<'a, 'b, L, I>
{
    type Item = &'b mut I;

    fn next(&mut self) -> Option<Self::Item> {
        let (now, remain) = self.bricks.split_first()?;
        self.bricks = remain;
        unsafe {
            Some(&mut *self.col_ptr.as_ptr().cast::<I>().add(*now))
        }
    }
}

// lower: newest
// higher: oldest

impl<const L: usize> Bricks<L> {
    pub fn new() -> Self {
        let mut idx_buf = [0; L];
        idx_buf.iter_mut().enumerate().for_each(|(i, v)| *v = i);
        Self {
            idx_buf: idx_buf,
            user_editable_end: 0,
            inco_editable_end: 0,
            history_end: 0,
        }
    }

    pub fn iter_user_editable<'a, I>(&'a self, t: &'a [I]) -> BrickIter<'a, L, I> {
        BrickIter {
            bricks: &self.idx_buf[0..self.user_editable_end],
            collection: t,
        }
    }

    pub fn iter_inco_editable<'a, I>(&'a self, t: &'a [I]) -> BrickIter<'a, L, I> {
        BrickIter {
            bricks: &self.idx_buf[self.user_editable_end..self.inco_editable_end],
            collection: t,
        }
    }

    pub fn iter_user_editable_mut<'a, 'b, I>(&'a self, t: &'b mut [I]) -> BrickIterMut<'a, 'b, L, I> {
        BrickIterMut {
            bricks: &self.idx_buf[0..self.user_editable_end],
            col_ptr: NonNull::from(t),
            _cpd: PhantomData,
        }
    }

    pub fn iter_inco_editable_mut<'a, 'b, I>(&'a self, t: &'b mut [I]) -> BrickIterMut<'a, 'b, L, I> {
        BrickIterMut {
            bricks: &self.idx_buf[self.user_editable_end..self.inco_editable_end],
            col_ptr: NonNull::from(t),
            _cpd: PhantomData,
        }
    }

    pub fn iter_history<'a, I>(&'a self, t: &'a [I]) -> BrickIter<'a, L, I> {
        BrickIter {
            bricks: &self.idx_buf[self.inco_editable_end..self.history_end],
            collection: t,
        }
    }

    pub fn pop_ue_front(&mut self) {
        if self.user_editable_end == 0 {
            return;
        }
        let end = self.history_end.wrapping_add(1).min(L);
        rot_left(&mut self.idx_buf[..end]);
        self.user_editable_end -= 1;
        self.inco_editable_end -= 1;
        self.history_end -= 1;
    }

    pub fn ue_front(&self) -> Option<usize> {
        if self.user_editable_end == 0 {
            None
        } else {
            Some(self.idx_buf[0])
        }
    }

    pub fn ie_front(&self) -> Option<usize> {
        if self.inco_editable_end == self.user_editable_end {
            None
        } else {
            Some(self.idx_buf[self.user_editable_end])
        }
    }

    // Operations:
    //
    // * Insert user editable -> Fails if all items already UE
    // * Insert inco editable -> Fails if all items already UE + IE
    // * Insert history       -> Fails if all items already UE + IE (not + history!)
    pub fn insert_ue_front(&mut self) -> Result<usize, ()> {
        if self.user_editable_end == L {
            return Err(());
        }
        // Rotate in at least one free/history
        let end = self.history_end.wrapping_add(1).min(L);
        rot_right(&mut self.idx_buf[..end]);
        self.user_editable_end = self.user_editable_end.wrapping_add(1).min(L);
        self.inco_editable_end = self.inco_editable_end.wrapping_add(1).min(L);
        self.history_end = self.history_end.wrapping_add(1).min(L);
        Ok(self.idx_buf[0])
    }

    pub fn insert_ie_front(&mut self) -> Result<usize, ()> {
        if self.inco_editable_end == L {
            return Err(());
        }
        // Rotate in at least one free/history
        let end = self.history_end.wrapping_add(1).min(L);
        rot_right(&mut self.idx_buf[self.user_editable_end..end]);
        self.inco_editable_end = self.inco_editable_end.wrapping_add(1).min(L);
        self.history_end = self.history_end.wrapping_add(1).min(L);
        Ok(self.idx_buf[self.user_editable_end])
    }

    pub fn release_ue(&mut self) {
        // We want to swap ue and ie regions.
        let range = &mut self.idx_buf[..self.inco_editable_end];

        // TODO(AJM): This is memory-friendly (requires only 1xusize extra),
        // but VERY CPU-unfriendly O(n^2) copies. This can be mitigated by
        // keeping the number of inco_editable items low, ideally 0/1.
        //
        // Alternatively, I could use O(n) extra storage, and assemble
        // the output directly.
        for _ in 0..(self.inco_editable_end - self.user_editable_end) {
            rot_right(range);
        }
        self.inco_editable_end -= self.user_editable_end;
        self.user_editable_end = 0;
    }

    pub fn release_ie(&mut self) {
        self.inco_editable_end = self.user_editable_end;
    }
}

#[cfg(test)]
pub mod test {
    use super::Bricks;

    #[test]
    fn smoke() {
        let mut brick = Bricks::<8>::new();
        println!("{:?}", brick);
        for i in 0..8 {
            let x = brick.insert_ue_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        println!("{:?}", brick);
        brick.insert_ue_front().unwrap_err();
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [7, 6, 5, 4, 3, 2, 1, 0],
                user_editable_end: 8,
                inco_editable_end: 8,
                history_end: 8,
            }
        );
        println!("=====");
        let mut brick = Bricks::<8>::new();
        for i in 0..4 {
            let x = brick.insert_ue_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [3, 2, 1, 0, 4, 5, 6, 7],
                user_editable_end: 4,
                inco_editable_end: 4,
                history_end: 4,
            }
        );
        println!("-----");
        for i in 4..8 {
            let x = brick.insert_ie_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [3, 2, 1, 0, 7, 6, 5, 4],
                user_editable_end: 4,
                inco_editable_end: 8,
                history_end: 8,
            }
        );
        println!("{:?}", brick);
        println!("=====");
        let mut brick = Bricks::<8>::new();
        for i in 0..3 {
            let x = brick.insert_ue_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        for i in 3..5 {
            let x = brick.insert_ie_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        println!("{:?}", brick);
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [2, 1, 0, 4, 3, 5, 6, 7],
                user_editable_end: 3,
                inco_editable_end: 5,
                history_end: 5,
            }
        );
        println!("-----");
        brick.release_ue();
        println!("{:?}", brick);
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [4, 3, 2, 1, 0, 5, 6, 7],
                user_editable_end: 0,
                inco_editable_end: 2,
                history_end: 5,
            }
        );
        println!("-----");
        brick.release_ie();
        println!("{:?}", brick);
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [4, 3, 2, 1, 0, 5, 6, 7],
                user_editable_end: 0,
                inco_editable_end: 0,
                history_end: 5,
            }
        );
        println!("=====");
        for i in 5..8 {
            let x = brick.insert_ue_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        println!("{:?}", brick);
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [7, 6, 5, 4, 3, 2, 1, 0],
                user_editable_end: 3,
                inco_editable_end: 3,
                history_end: 8,
            }
        );
        println!("-----");
        for i in 0..2 {
            let x = brick.insert_ie_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        println!("{:?}", brick);
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [7, 6, 5, 1, 0, 4, 3, 2],
                user_editable_end: 3,
                inco_editable_end: 5,
                history_end: 8,
            }
        );


        let mut buf = [10, 20, 30, 40, 50, 60, 70, 80];
        assert_eq!(
            brick.iter_user_editable(&buf).copied().collect::<Vec<_>>().as_slice(),
            &[80, 70, 60],
        );
        assert_eq!(
            brick.iter_user_editable_mut(&mut buf).map(|c| *c).collect::<Vec<_>>().as_slice(),
            &[80, 70, 60],
        );
        assert_eq!(
            brick.iter_inco_editable(&buf).copied().collect::<Vec<_>>().as_slice(),
            &[20, 10],
        );
        assert_eq!(
            brick.iter_inco_editable_mut(&mut buf).map(|c| *c).collect::<Vec<_>>().as_slice(),
            &[20, 10],
        );
        assert_eq!(
            brick.iter_history(&buf).copied().collect::<Vec<_>>().as_slice(),
            &[50, 40, 30],
        );

        println!("-----");
        for i in 2..5 {
            let x = brick.insert_ue_front().unwrap();
            println!("{:?}", brick);
            assert_eq!(x, i);
        }
        println!("{:?}", brick);
        assert_eq!(
            brick,
            Bricks {
                idx_buf: [4, 3, 2, 7, 6, 5, 1, 0],
                user_editable_end: 6,
                inco_editable_end: 8,
                history_end: 8,
            }
        );
        brick.insert_ie_front().unwrap_err();
        assert_eq!(brick.insert_ue_front().unwrap(), 0);



    }
}


