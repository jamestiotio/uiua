use std::{
    cmp::Ordering,
    collections::{BTreeMap, BTreeSet},
    iter::repeat,
    mem::{swap, take},
    ptr,
};

use crate::{
    array::{Array, ArrayType},
    value::{Type, Value},
    vm::Env,
    RuntimeResult,
};

type CmpFn<T> = fn(&T, &T) -> Ordering;

impl Value {
    #[allow(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        if self.is_array() {
            self.array().len()
        } else {
            1
        }
    }
    pub fn rank(&self) -> usize {
        if self.is_array() {
            self.array().rank()
        } else {
            0
        }
    }
    pub fn shape(&self) -> Vec<usize> {
        if self.is_array() {
            self.array().shape().to_vec()
        } else {
            Vec::new()
        }
    }
    pub fn as_shape(&self, env: &Env, error: &'static str) -> RuntimeResult<Vec<usize>> {
        if self.is_array() {
            let arr = self.array();
            let numbers = if arr.is_numbers() {
                arr.numbers()
            } else if arr.shape() == [0] {
                &[]
            } else {
                return Err(env.error(error));
            };
            let mut shape = Vec::with_capacity(arr.len());
            for f in numbers {
                let rounded = f.round();
                if (f - rounded).abs() > f64::EPSILON || rounded <= 0.0 {
                    return Err(env.error(error));
                }
                let rounded = rounded as usize;
                shape.push(rounded);
            }
            Ok(shape)
        } else if self.is_num() {
            let f = self.number();
            let rounded = f.round();
            if (f - rounded).abs() > f64::EPSILON || rounded <= 0.0 {
                return Err(env.error(error));
            }
            Ok(vec![rounded as usize])
        } else {
            return Err(env.error(error));
        }
    }
    pub fn as_indices(&self, env: &Env, error: &'static str) -> RuntimeResult<Vec<isize>> {
        self.as_number_list(env, error, |f| f % 1.0 == 0.0, |f| f as isize)
    }
    pub fn as_positives(&self, env: &Env, error: &'static str) -> RuntimeResult<Vec<usize>> {
        self.as_number_list(env, error, |f| f % 1.0 == 0.0 && f >= 0.0, |f| f as usize)
    }
    fn as_number_list<T>(
        &self,
        env: &Env,
        error: &'static str,
        test: fn(f64) -> bool,
        convert: fn(f64) -> T,
    ) -> RuntimeResult<Vec<T>> {
        if self.is_array() {
            let arr = self.array();
            let numbers = if arr.is_numbers() {
                arr.numbers()
            } else if arr.shape() == [0] {
                &[]
            } else {
                return Err(env.error(error));
            };
            let mut index = Vec::with_capacity(arr.len());
            for f in numbers {
                if !test(*f) {
                    return Err(env.error(error));
                }
                index.push(convert(*f));
            }
            Ok(index)
        } else if self.is_num() {
            let f = self.number();
            if !test(f) {
                return Err(env.error(error));
            }
            Ok(vec![convert(f)])
        } else {
            return Err(env.error(error));
        }
    }
    pub fn range(&mut self, env: &Env) -> RuntimeResult {
        let shape = self.as_shape(
            env,
            "Range only accepts a single natural number \
            or a list of natural numbers",
        )?;
        let data = range(&shape);
        *self = Array::from((shape, data)).into();
        Ok(())
    }
    pub fn reverse(&mut self) {
        if self.is_array() {
            self.array_mut().reverse();
        }
    }
    pub fn join(&mut self, other: Value, env: &Env) -> RuntimeResult {
        match (self.is_array(), other.is_array()) {
            (true, true) => self.array_mut().join(other.into_array(), env)?,
            (true, false) => self.array_mut().join(Array::from(other), env)?,
            (false, true) => {
                let mut arr = Array::from(take(self));
                arr.join(other.into_array(), env)?;
                *self = arr.into();
            }
            (false, false) => {
                let mut arr = Array::from(take(self));
                arr.join(Array::from(other), env)?;
                *self = arr.into();
            }
        }
        self.array_mut().normalize(0);
        Ok(())
    }
    pub fn deshape(&mut self) {
        if self.is_array() {
            self.array_mut().deshape();
        } else {
            *self = Array::from(take(self)).into();
        }
    }
    pub fn reshape(&mut self, mut other: Value, env: &Env) -> RuntimeResult {
        swap(self, &mut other);
        let shape = other.as_shape(env, "Shape must be a list of natural numbers")?;
        self.coerce_array().reshape(shape);
        Ok(())
    }
    pub fn coerce_array(&mut self) -> &mut Array {
        if !self.is_array() {
            *self = match self.ty() {
                Type::Num => Array::from(self.number()),
                Type::Char => Array::from(self.char()),
                _ => Array::from(take(self)),
            }
            .into();
        }
        self.array_mut()
    }
    pub fn coerce_into_array(mut self) -> Array {
        self.coerce_array();
        self.into_array()
    }
    pub fn replicate(&mut self, items: Self, env: &Env) -> RuntimeResult {
        if !items.is_array() {
            return Err(env.error("Cannot filter non-array"));
        }
        let filtered = items.into_array();
        let mut data = Vec::new();
        if self.is_num() {
            if !self.is_nat() {
                return Err(env.error("Cannot replicate with non-integer"));
            }
            let n = self.number() as usize;
            for cell in filtered.into_values() {
                data.extend(repeat(cell).take(n));
            }
        } else if self.is_array() {
            let filter = self.array();
            if filter.len() != filtered.len() {
                return Err(env.error(format!(
                    "Cannot replicate with array of different length: \
                    the filter length is {}, but the array length is {}",
                    filter.len(),
                    filtered.len(),
                )));
            }
            if !filter.is_numbers() {
                return Err(env.error("Cannot replicate with non-number array"));
            }
            if filter.rank() != 1 {
                return Err(env.error("Cannot replicate with non-1D array"));
            }
            for (&n, cell) in filter.numbers().iter().zip(filtered.into_values()) {
                if n.trunc() != n || n < 0.0 {
                    return Err(env.error("Cannot replicate with non-natural number"));
                }
                data.extend(repeat(cell).take(n as usize));
            }
        } else {
            return Err(env.error("Cannot replicate with non-number"));
        }
        *self = Array::from(data).normalized(1).into();
        Ok(())
    }
    pub fn pick(&mut self, from: Self, env: &Env) -> RuntimeResult {
        if !from.is_array() || from.array().rank() == 0 {
            return Err(env.error("Cannot pick from rank less than 1"));
        }
        let index = self.as_indices(env, "Index must be a list of integers")?;
        let array = from.array();
        *self = pick(&index, array, env)?;
        Ok(())
    }
    pub fn first(&mut self, env: &Env) -> RuntimeResult {
        if !self.is_array() {
            return Ok(());
        }
        let array = take(self.array_mut());
        *self = array
            .into_first()
            .ok_or_else(|| env.error("Empty array has no first"))?;
        Ok(())
    }
    pub fn take(&mut self, from: Self, env: &Env) -> RuntimeResult {
        if !from.is_array() || from.array().rank() == 0 {
            return Err(env.error("Cannot take from rank less than 1"));
        }
        let index = self.as_indices(env, "Index must be a list of integers")?;
        let array = from.into_array();
        if index.len() > array.rank() {
            return Err(env.error(format!(
                "Cannot take with index of greater rank: \
                the index length is {}, but the array rank is {}",
                index.len(),
                array.rank(),
            )));
        }
        let taken = take_array(&index, array, env)?;
        *self = taken.into();
        Ok(())
    }
    pub fn drop(&mut self, from: Self, env: &Env) -> RuntimeResult {
        if !from.is_array() || from.array().rank() == 0 {
            return Err(env.error("Cannot drop from rank less than 1"));
        }
        let mut index = self.as_indices(env, "Index must be a list of integers")?;
        let array = from.into_array();
        if index.len() > array.rank() {
            return Err(env.error(format!(
                "Cannot drop with index of greater rank: \
                the index length is {}, but the array rank is {}",
                index.len(),
                array.rank(),
            )));
        }
        for (i, s) in index.iter_mut().zip(array.shape()) {
            *i = if *i >= 0 {
                (*i - (*s as isize)).min(0)
            } else {
                ((*s as isize) + *i).max(0)
            };
        }
        let taken = take_array(&index, array, env)?;
        *self = taken.into();
        Ok(())
    }
    pub fn fill_value(&self, env: &Env) -> RuntimeResult<Value> {
        Ok(match self.ty() {
            Type::Num => 0.0.into(),
            Type::Char => ' '.into(),
            Type::Function => return Err(env.error("Functions do not have a fill value")),
            Type::Array => {
                let array = self.array();
                let values: Vec<Value> = array
                    .clone()
                    .into_values()
                    .into_iter()
                    .map(|val| val.fill_value(env))
                    .collect::<RuntimeResult<_>>()?;
                Array::from((array.shape().to_vec(), values))
                    .normalized(1)
                    .into()
            }
        })
    }
    pub fn rotate(&mut self, mut target: Self, env: &Env) -> RuntimeResult {
        swap(self, &mut target);
        let index = target.as_indices(env, "Index must be a list of integers")?;
        if index.is_empty() || index.iter().all(|i| *i == 0) {
            return Ok(());
        }
        if !self.is_array() || self.array().shape() == [0] {
            return Ok(());
        }
        self.array_mut().data_mut(
            |shape, data| rotate(&index, shape, data),
            |shape, data| rotate(&index, shape, data),
            |shape, data| rotate(&index, shape, data),
        );
        Ok(())
    }
    pub fn transpose(&mut self) {
        let arr = self.coerce_array();
        arr.data_mut(transpose, transpose, transpose);
    }
    pub fn enclose(&mut self) {
        *self = Array::from((Vec::new(), vec![take(self)]))
            .normalized(0)
            .into();
    }
    pub fn pair(&mut self, other: Self) {
        *self = Array::from((vec![2], vec![take(self), other]))
            .normalized(0)
            .into();
    }
    pub fn couple(&mut self, mut other: Self, env: &Env) -> RuntimeResult {
        let a = self.coerce_array();
        let b = other.coerce_array();
        if a.shape() != b.shape() {
            return Err(env.error(format!(
                "Cannot couple arrays of different shapes: \
                the first shape is {:?}, but the second shape is {:?}",
                a.shape(),
                b.shape()
            )));
        }
        match (a.ty(), b.ty()) {
            (ArrayType::Num, ArrayType::Num) => a.numbers_mut().append(b.numbers_mut()),
            (ArrayType::Char, ArrayType::Char) => a.chars_mut().append(b.chars_mut()),
            (ArrayType::Value, ArrayType::Value) => a.values_mut().append(b.values_mut()),
            _ => a.make_values().append(b.make_values()),
        }
        a.shape_mut().insert(0, 2);
        Ok(())
    }
    pub fn grade(&mut self, env: &Env) -> RuntimeResult {
        let arr = self.coerce_array();
        if arr.rank() < 1 {
            return Err(env.error("Cannot grade rank less than 1"));
        }
        let mut indices: Vec<usize> = (0..arr.shape()[0]).collect();
        let cells = take(arr).into_values();
        indices.sort_by(|&a, &b| cells[a].cmp(&cells[b]));
        let nums: Vec<f64> = indices.iter().map(|&i| i as f64).collect();
        *arr = Array::from((vec![indices.len()], nums));
        Ok(())
    }
    pub fn select(&mut self, mut from: Self, env: &Env) -> RuntimeResult {
        let indices = self.as_indices(env, "Indices must be a list of integers")?;
        let array = from.coerce_array();
        let mut selected = Vec::with_capacity(indices.len());
        for index in indices {
            selected.push(pick(&[index], array, env)?);
        }
        *self = Array::from((vec![selected.len()], selected))
            .normalized(1)
            .into();
        Ok(())
    }
    pub fn windows(&mut self, from: Self, env: &Env) -> RuntimeResult {
        let mut array = from.coerce_into_array();
        let sizes = self.as_positives(env, "Window size must be a list of positive integers")?;
        if sizes.is_empty() {
            return Ok(());
        }
        array_windows(&sizes, &mut array, env)?;
        *self = array.into();
        Ok(())
    }
    pub fn classify(&mut self, env: &Env) -> RuntimeResult {
        if self.rank() < 1 {
            return Err(env.error("Cannot classify rank less than 1"));
        }
        let array = take(self).into_array();
        let mut classes = BTreeMap::new();
        let mut classified = Vec::with_capacity(array.shape()[0]);
        for val in array.into_values() {
            let new_class = classes.len();
            let class = *classes.entry(val).or_insert(new_class);
            classified.push(class as f64);
        }
        *self = Array::from(classified).into();
        Ok(())
    }
    pub fn member(&mut self, of: Self) {
        let members = self.coerce_array();
        let set: BTreeSet<Value> = of.coerce_into_array().into_values().into_iter().collect();
        *self = Array::from(
            take(members)
                .into_values()
                .into_iter()
                .map(|val| set.contains(&val) as u8 as f64)
                .collect::<Vec<_>>(),
        )
        .into();
    }
}

fn array_windows(mut sizes: &[usize], array: &mut Array, env: &Env) -> RuntimeResult {
    if sizes.is_empty() {
        return Ok(());
    }
    let window_size = sizes[0];
    sizes = &sizes[1..];
    let window_count = if window_size <= array.shape()[0] {
        array.shape()[0] - window_size + 1
    } else {
        return Err(env.error(format!(
            "Window size of {} is too large for shape {:?}",
            window_size,
            array.shape(),
        )));
    };
    let mut windows = Vec::with_capacity(window_count);
    let mut window_shape = array.shape().to_vec();
    let cells = take(array).into_cells();
    window_shape[0] = window_count;
    for window in cells.windows(window_size) {
        let mut window = window.to_vec();
        for array in &mut window {
            array_windows(sizes, array, env)?;
        }
        windows.push(Array::from(window));
    }
    *array = Array::from(windows);
    Ok(())
}

fn transpose<T: Clone>(shape: &mut [usize], data: &mut [T]) {
    if shape.len() < 2 || shape[0] == 0 {
        return;
    }
    let mut temp = Vec::with_capacity(data.len());
    let run_length = data.len() / shape[0];
    for j in 0..run_length {
        for i in 0..shape[0] {
            temp.push(data[i * run_length + j].clone());
        }
    }
    data.clone_from_slice(&temp);
    shape.rotate_left(1);
}

fn rotate<T: Clone>(index: &[isize], shape: &[usize], data: &mut [T]) {
    let cell_count = shape[0];
    if cell_count == 0 {
        return;
    }
    let cell_size = data.len() / cell_count;
    let offset = index[0];
    let mid = (cell_count as isize + offset).rem_euclid(cell_count as isize) as usize;
    let (left, right) = data.split_at_mut(mid * cell_size);
    left.reverse();
    right.reverse();
    data.reverse();
    let index = &index[1..];
    let shape = &shape[1..];
    if index.is_empty() || shape.is_empty() {
        return;
    }
    for cell in data.chunks_mut(cell_size) {
        rotate(index, shape, cell);
    }
}

fn take_array(index: &[isize], array: Array, env: &Env) -> RuntimeResult<Array> {
    let mut shape = array.shape().to_vec();
    let mut cells = array.into_values();
    let take_count = index[0];
    let take_abs = take_count.unsigned_abs();
    if take_count >= 0 {
        cells.truncate(take_abs);
        if cells.len() < take_abs {
            let fill = cells[0].fill_value(env)?;
            cells.extend(repeat(fill).take(take_abs - cells.len()));
        }
    } else {
        if cells.len() > take_abs {
            cells.drain(0..cells.len() - take_abs);
        }
        if cells.len() < take_abs {
            let fill = cells[0].fill_value(env)?;
            cells = repeat(fill)
                .take(take_abs - cells.len())
                .chain(cells)
                .collect();
        }
    }
    let index = &index[1..];
    shape[0] = take_abs;
    if index.is_empty() {
        let norm = if shape.len() > 1 { 1 } else { 0 };
        Ok(Array::from((shape, cells)).normalized(norm))
    } else {
        cells = cells
            .into_iter()
            .map(|cell| take_array(index, cell.into_array(), env).map(Value::from))
            .collect::<RuntimeResult<_>>()?;
        Ok(Array::from((shape, cells)).normalized(1))
    }
}

fn pick(index: &[isize], array: &Array, env: &Env) -> RuntimeResult<Value> {
    if index.len() > array.rank() {
        return Err(env.error(format!(
            "Cannot pick with index of greater rank: \
                the index length is {}, but the array rank is {}",
            index.len(),
            array.rank(),
        )));
    }
    for (&s, &i) in array.shape().iter().zip(index) {
        let s = s as isize;
        if i >= s || s + i < 0 {
            return Err(env.error(format!(
                "Index out of range: \
                    the index is {:?}, but the shape is {:?}",
                index,
                array.shape()
            )));
        }
    }
    Ok(match array.ty() {
        ArrayType::Num => pick_impl(array.shape(), index, array.numbers()),
        ArrayType::Char => pick_impl(array.shape(), index, array.chars()),
        ArrayType::Value => pick_impl(array.shape(), index, array.values()),
    })
}

fn pick_impl<T>(shape: &[usize], index: &[isize], mut data: &[T]) -> Value
where
    T: Clone + Into<Value>,
    Array: From<(Vec<usize>, Vec<T>)>,
{
    let mut shape_index = 0;
    for &i in index {
        let cell_count = shape[shape_index];
        let cell_size = data.len() / cell_count;
        let start = if i >= 0 {
            i as usize * cell_size
        } else {
            (data.len() as isize + i * cell_size as isize) as usize
        };
        data = &data[start..start + cell_size];
        shape_index += 1;
    }
    if shape_index < shape.len() {
        let shape = shape[shape_index..].to_vec();
        Array::from((shape, data.to_vec())).into()
    } else {
        data[0].clone().into()
    }
}

pub fn range(shape: &[usize]) -> Vec<Value> {
    let len = shape.iter().product::<usize>();
    let mut data = Vec::with_capacity(len);
    let products: Vec<usize> = (0..shape.len())
        .map(|i| shape[i..].iter().product::<usize>())
        .collect();
    let moduli: Vec<usize> = (0..shape.len())
        .map(|i| shape[i + 1..].iter().product::<usize>())
        .collect();
    for i in 0..len {
        if shape.len() <= 1 {
            data.push((i as f64).into());
        } else {
            let mut cell: Vec<f64> = Vec::with_capacity(shape.len());
            for j in 0..shape.len() {
                cell.push((i % products[j] / moduli[j]) as f64);
            }
            data.push(Array::from(cell).into());
        }
    }
    data
}

pub fn reverse<T>(shape: &[usize], data: &mut [T]) {
    if shape.is_empty() {
        return;
    }
    let cells = shape[0];
    let cell_size: usize = shape.iter().skip(1).product();
    for i in 0..cells / 2 {
        let left = i * cell_size;
        let right = (cells - i - 1) * cell_size;
        let left = &mut data[left] as *mut T;
        let right = &mut data[right] as *mut T;
        unsafe {
            ptr::swap_nonoverlapping(left, right, cell_size);
        }
    }
}

pub fn force_length<T: Clone>(data: &mut Vec<T>, len: usize) {
    match data.len().cmp(&len) {
        Ordering::Less => {
            let mut i = 0;
            while data.len() < len {
                data.push(data[i].clone());
                i += 1;
            }
        }
        Ordering::Greater => data.truncate(len),
        Ordering::Equal => {}
    }
}

pub fn sort_array<T: Clone>(shape: &[usize], data: &mut [T], cmp: CmpFn<T>) {
    if shape.is_empty() {
        return;
    }
    let chunk_size = shape.iter().skip(1).product();
    merge_sort_chunks(chunk_size, data, cmp);
}

fn merge_sort_chunks<T: Clone>(chunk_size: usize, data: &mut [T], cmp: CmpFn<T>) {
    let cells = data.len() / chunk_size;
    assert_ne!(cells, 0);
    if cells == 1 {
        return;
    }
    let mid = cells / 2;
    let mut tmp = Vec::with_capacity(data.len());
    let (left, right) = data.split_at_mut(mid * chunk_size);
    merge_sort_chunks(chunk_size, left, cmp);
    merge_sort_chunks(chunk_size, right, cmp);
    let mut left = left.chunks_exact(chunk_size);
    let mut right = right.chunks_exact(chunk_size);
    let mut left_next = left.next();
    let mut right_next = right.next();
    loop {
        match (left_next, right_next) {
            (Some(l), Some(r)) => {
                let mut ordering = Ordering::Equal;
                for (l, r) in l.iter().zip(r) {
                    ordering = cmp(l, r);
                    if ordering != Ordering::Equal {
                        break;
                    }
                }
                if ordering == Ordering::Less {
                    tmp.extend_from_slice(l);
                    left_next = left.next();
                } else {
                    tmp.extend_from_slice(r);
                    right_next = right.next();
                }
            }
            (Some(l), None) => {
                tmp.extend_from_slice(l);
                left_next = left.next();
            }
            (None, Some(r)) => {
                tmp.extend_from_slice(r);
                right_next = right.next();
            }
            (None, None) => {
                break;
            }
        }
    }
    data.clone_from_slice(&tmp);
}
