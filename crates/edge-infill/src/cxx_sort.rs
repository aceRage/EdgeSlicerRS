//! The order libc++'s `std::sort` leaves equal elements in.
//!
//! `std::sort` does not keep equal elements in the order they came, and
//! where the reference sorts things that can tie (the arcs between line
//! ends in `connect_infill`, which on a square part come out exactly the
//! same length again and again) the order it leaves the ties in decides
//! which ends are joined. To come out the same, this is a port of the
//! introsort of LLVM's libc++ (`__algorithm/sort.h`, `__introsort` and the
//! helpers it uses on a type that is not arithmetic), which the macOS
//! reference build is compiled against. A build of the reference with
//! another standard library (MSVC's on Windows) may break such ties the
//! other way; that is the reference's own doing.
//!
//! The heap sort `__introsort` falls back on after `2 log2 n` bad pivots
//! is not ported: the slices sorted here are a few hundred elements of
//! lengths, far from that, and a stable sort stands in for it.

/// Sorts `v` as libc++'s `std::sort(v.begin(), v.end(), less)` does.
pub(crate) fn sort_by<T: Copy>(v: &mut [T], less: impl Fn(&T, &T) -> bool) {
    let n = v.len();
    if n == 0 {
        return;
    }
    let depth = 2 * (usize::BITS - 1 - n.leading_zeros()) as i64;
    introsort(v, 0, n, &less, depth, true);
}

fn sort3<T: Copy>(
    v: &mut [T],
    x: usize,
    y: usize,
    z: usize,
    less: &impl Fn(&T, &T) -> bool,
) -> bool {
    if !less(&v[y], &v[x]) {
        if !less(&v[z], &v[y]) {
            return false;
        }
        v.swap(y, z);
        if less(&v[y], &v[x]) {
            v.swap(x, y);
        }
        return true;
    }
    if less(&v[z], &v[y]) {
        v.swap(x, z);
        return true;
    }
    v.swap(x, y);
    if less(&v[z], &v[y]) {
        v.swap(y, z);
    }
    true
}

fn sort4<T: Copy>(
    v: &mut [T],
    x1: usize,
    x2: usize,
    x3: usize,
    x4: usize,
    less: &impl Fn(&T, &T) -> bool,
) {
    sort3(v, x1, x2, x3, less);
    if less(&v[x4], &v[x3]) {
        v.swap(x3, x4);
        if less(&v[x3], &v[x2]) {
            v.swap(x2, x3);
            if less(&v[x2], &v[x1]) {
                v.swap(x1, x2);
            }
        }
    }
}

fn sort5<T: Copy>(v: &mut [T], x: [usize; 5], less: &impl Fn(&T, &T) -> bool) {
    sort4(v, x[0], x[1], x[2], x[3], less);
    if less(&v[x[4]], &v[x[3]]) {
        v.swap(x[3], x[4]);
        if less(&v[x[3]], &v[x[2]]) {
            v.swap(x[2], x[3]);
            if less(&v[x[2]], &v[x[1]]) {
                v.swap(x[1], x[2]);
                if less(&v[x[1]], &v[x[0]]) {
                    v.swap(x[0], x[1]);
                }
            }
        }
    }
}

fn insertion_sort<T: Copy>(v: &mut [T], first: usize, last: usize, less: &impl Fn(&T, &T) -> bool) {
    if first == last {
        return;
    }
    for i in first + 1..last {
        let mut j = i - 1;
        if less(&v[i], &v[j]) {
            let t = v[i];
            let mut k = j;
            j = i;
            loop {
                v[j] = v[k];
                j = k;
                if j == first {
                    break;
                }
                k -= 1;
                if !less(&t, &v[k]) {
                    break;
                }
            }
            v[j] = t;
        }
    }
}

/// As [`insertion_sort`], relying on an element before `first` that is
/// no greater than any in the range.
fn insertion_sort_unguarded<T: Copy>(
    v: &mut [T],
    first: usize,
    last: usize,
    less: &impl Fn(&T, &T) -> bool,
) {
    if first == last {
        return;
    }
    for i in first + 1..last {
        let mut j = i - 1;
        if less(&v[i], &v[j]) {
            let t = v[i];
            let mut k = j;
            j = i;
            loop {
                v[j] = v[k];
                j = k;
                k -= 1;
                if !less(&t, &v[k]) {
                    break;
                }
            }
            v[j] = t;
        }
    }
}

fn insertion_sort_incomplete<T: Copy>(
    v: &mut [T],
    first: usize,
    last: usize,
    less: &impl Fn(&T, &T) -> bool,
) -> bool {
    match last - first {
        0 | 1 => return true,
        2 => {
            if less(&v[last - 1], &v[first]) {
                v.swap(first, last - 1);
            }
            return true;
        }
        3 => {
            sort3(v, first, first + 1, last - 1, less);
            return true;
        }
        4 => {
            sort4(v, first, first + 1, first + 2, last - 1, less);
            return true;
        }
        5 => {
            sort5(v, [first, first + 1, first + 2, first + 3, last - 1], less);
            return true;
        }
        _ => {}
    }
    let mut j = first + 2;
    sort3(v, first, first + 1, j, less);
    let limit = 8;
    let mut count = 0;
    let mut i = j + 1;
    while i != last {
        if less(&v[i], &v[j]) {
            let t = v[i];
            let mut k = j;
            j = i;
            loop {
                v[j] = v[k];
                j = k;
                if j == first {
                    break;
                }
                k -= 1;
                if !less(&t, &v[k]) {
                    break;
                }
            }
            v[j] = t;
            count += 1;
            if count == limit {
                return i + 1 == last;
            }
        }
        j = i;
        i += 1;
    }
    true
}

fn partition_with_equals_on_right<T: Copy>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    less: &impl Fn(&T, &T) -> bool,
) -> (usize, bool) {
    let begin = first;
    let pivot = v[first];
    loop {
        first += 1;
        if !less(&v[first], &pivot) {
            break;
        }
    }
    if begin == first - 1 {
        while first < last {
            last -= 1;
            if less(&v[last], &pivot) {
                break;
            }
        }
    } else {
        loop {
            last -= 1;
            if less(&v[last], &pivot) {
                break;
            }
        }
    }
    let already = first >= last;
    while first < last {
        v.swap(first, last);
        loop {
            first += 1;
            if !less(&v[first], &pivot) {
                break;
            }
        }
        loop {
            last -= 1;
            if less(&v[last], &pivot) {
                break;
            }
        }
    }
    let pivot_pos = first - 1;
    if begin != pivot_pos {
        v[begin] = v[pivot_pos];
    }
    v[pivot_pos] = pivot;
    (pivot_pos, already)
}

fn partition_with_equals_on_left<T: Copy>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    less: &impl Fn(&T, &T) -> bool,
) -> usize {
    let begin = first;
    let pivot = v[first];
    if less(&pivot, &v[last - 1]) {
        loop {
            first += 1;
            if less(&pivot, &v[first]) {
                break;
            }
        }
    } else {
        loop {
            first += 1;
            if !(first < last && !less(&pivot, &v[first])) {
                break;
            }
        }
    }
    if first < last {
        loop {
            last -= 1;
            if !less(&pivot, &v[last]) {
                break;
            }
        }
    }
    while first < last {
        v.swap(first, last);
        loop {
            first += 1;
            if less(&pivot, &v[first]) {
                break;
            }
        }
        loop {
            last -= 1;
            if !less(&pivot, &v[last]) {
                break;
            }
        }
    }
    let pivot_pos = first - 1;
    if begin != pivot_pos {
        v[begin] = v[pivot_pos];
    }
    v[pivot_pos] = pivot;
    first
}

fn introsort<T: Copy>(
    v: &mut [T],
    mut first: usize,
    mut last: usize,
    less: &impl Fn(&T, &T) -> bool,
    mut depth: i64,
    mut leftmost: bool,
) {
    const LIMIT: usize = 24;
    const NINTHER: usize = 128;
    loop {
        let len = last - first;
        match len {
            0 | 1 => return,
            2 => {
                if less(&v[last - 1], &v[first]) {
                    v.swap(first, last - 1);
                }
                return;
            }
            3 => {
                sort3(v, first, first + 1, last - 1, less);
                return;
            }
            4 => {
                sort4(v, first, first + 1, first + 2, last - 1, less);
                return;
            }
            5 => {
                sort5(v, [first, first + 1, first + 2, first + 3, last - 1], less);
                return;
            }
            _ => {}
        }
        if len < LIMIT {
            if leftmost {
                insertion_sort(v, first, last, less);
            } else {
                insertion_sort_unguarded(v, first, last, less);
            }
            return;
        }
        if depth == 0 {
            // libc++ heap-sorts here; see the module's note.
            v[first..last].sort_by(|a, b| {
                if less(a, b) {
                    std::cmp::Ordering::Less
                } else if less(b, a) {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            });
            return;
        }
        depth -= 1;
        let half = len / 2;
        if len > NINTHER {
            sort3(v, first, first + half, last - 1, less);
            sort3(v, first + 1, first + half - 1, last - 2, less);
            sort3(v, first + 2, first + half + 1, last - 3, less);
            sort3(v, first + half - 1, first + half, first + half + 1, less);
            v.swap(first, first + half);
        } else {
            sort3(v, first + half, first, last - 1, less);
        }
        if !leftmost && !less(&v[first - 1], &v[first]) {
            first = partition_with_equals_on_left(v, first, last, less);
            continue;
        }
        let (i, already) = partition_with_equals_on_right(v, first, last, less);
        if already {
            let fs = insertion_sort_incomplete(v, first, i, less);
            if insertion_sort_incomplete(v, i + 1, last, less) {
                if fs {
                    return;
                }
                last = i;
                continue;
            } else if fs {
                first = i + 1;
                continue;
            }
        }
        introsort(v, first, i, less, depth, leftmost);
        leftmost = false;
        first = i + 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sorts_like_any_sort() {
        let mut seed = 12345u64;
        for n in [0usize, 1, 2, 3, 5, 7, 23, 24, 25, 60, 129, 300, 1000] {
            let mut v: Vec<(u32, usize)> = (0..n)
                .map(|i| {
                    seed = seed
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    (((seed >> 33) % 17) as u32, i)
                })
                .collect();
            let mut want: Vec<u32> = v.iter().map(|x| x.0).collect();
            want.sort();
            sort_by(&mut v, |a, b| a.0 < b.0);
            assert_eq!(v.iter().map(|x| x.0).collect::<Vec<_>>(), want, "n = {n}");
        }
    }

    /// What libc++'s `std::sort` gives, from a small C++ program built
    /// with Apple clang against the macOS SDK the reference is built with:
    /// keys alternating 0 and 1, and keys in 0..7 from the generator in
    /// [`sorts_like_any_sort`], each with its place as the second field.
    #[test]
    fn ties_come_out_as_libcxx_leaves_them() {
        let alternating = |n: usize| -> Vec<usize> {
            let mut v: Vec<(u32, usize)> = (0..n).map(|i| ((i % 2) as u32, i)).collect();
            sort_by(&mut v, |a, b| a.0 < b.0);
            v.iter().map(|x| x.1).collect()
        };
        assert_eq!(
            alternating(20),
            [
                0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 1, 3, 5, 7, 9, 11, 13, 15, 17, 19
            ]
        );
        assert_eq!(
            alternating(30),
            [
                0, 28, 2, 26, 4, 24, 6, 22, 8, 20, 10, 18, 12, 16, 14, 15, 13, 17, 11, 19, 9, 21,
                7, 23, 5, 25, 3, 27, 1, 29
            ]
        );
        assert_eq!(
            alternating(60),
            [
                0, 2, 4, 6, 8, 10, 12, 14, 16, 18, 20, 22, 24, 26, 28, 30, 32, 34, 58, 56, 54, 52,
                50, 48, 46, 44, 42, 40, 38, 36, 27, 59, 29, 31, 33, 35, 23, 37, 21, 39, 19, 41, 17,
                43, 15, 45, 13, 47, 11, 49, 9, 51, 7, 53, 5, 55, 3, 57, 1, 25
            ]
        );
        assert_eq!(
            alternating(200),
            [
                0, 198, 2, 196, 4, 194, 6, 192, 8, 190, 10, 188, 12, 186, 14, 184, 16, 182, 18,
                180, 20, 178, 22, 176, 24, 174, 26, 172, 28, 170, 30, 168, 32, 166, 34, 164, 36,
                162, 38, 160, 40, 158, 42, 156, 44, 154, 46, 152, 48, 150, 50, 148, 52, 146, 54,
                144, 56, 142, 58, 140, 60, 138, 62, 136, 64, 134, 66, 132, 68, 130, 70, 128, 72,
                126, 74, 124, 76, 122, 78, 120, 80, 118, 82, 116, 84, 114, 86, 112, 88, 110, 90,
                108, 92, 106, 94, 104, 96, 102, 98, 100, 1, 199, 97, 103, 95, 105, 93, 107, 91,
                109, 89, 111, 87, 113, 85, 115, 83, 117, 81, 119, 79, 121, 77, 123, 75, 125, 73,
                127, 71, 129, 69, 131, 67, 133, 65, 135, 63, 137, 61, 139, 59, 141, 57, 143, 55,
                145, 53, 147, 51, 149, 49, 151, 47, 153, 45, 155, 43, 157, 41, 159, 39, 161, 37,
                163, 35, 165, 33, 167, 31, 169, 29, 171, 27, 173, 25, 175, 23, 177, 21, 179, 19,
                181, 17, 183, 15, 185, 13, 187, 11, 189, 9, 191, 7, 193, 5, 195, 3, 197, 99, 101
            ]
        );
        let mut seed = 12345u64;
        let mut v: Vec<(u32, usize)> = (0..500)
            .map(|i| {
                seed = seed
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                (((seed >> 33) % 7) as u32, i)
            })
            .collect();
        sort_by(&mut v, |a, b| a.0 < b.0);
        let want: [usize; 500] = [
            152, 70, 251, 488, 486, 484, 9, 466, 462, 458, 24, 157, 158, 138, 160, 136, 170, 174,
            184, 126, 186, 188, 122, 121, 203, 449, 209, 116, 115, 112, 420, 211, 217, 417, 226,
            106, 391, 377, 375, 228, 237, 247, 95, 94, 364, 266, 46, 84, 279, 282, 348, 286, 290,
            339, 54, 55, 294, 335, 75, 331, 329, 325, 295, 321, 319, 313, 496, 142, 297, 148, 149,
            298, 66, 72, 322, 332, 76, 336, 343, 288, 347, 49, 48, 83, 349, 276, 273, 271, 268,
            363, 258, 91, 257, 255, 366, 367, 96, 497, 370, 99, 244, 243, 371, 229, 372, 105, 392,
            397, 108, 223, 418, 419, 423, 210, 114, 427, 436, 438, 451, 119, 194, 21, 463, 465,
            124, 16, 469, 476, 128, 178, 13, 131, 171, 480, 164, 161, 481, 483, 487, 4, 489, 296,
            71, 69, 498, 219, 249, 2, 490, 5, 10, 482, 479, 478, 475, 472, 459, 25, 442, 159, 439,
            26, 433, 430, 428, 425, 422, 33, 413, 403, 401, 399, 35, 36, 362, 175, 352, 351, 53,
            56, 327, 181, 61, 183, 320, 185, 125, 309, 307, 77, 293, 291, 192, 287, 285, 82, 267,
            263, 261, 256, 254, 250, 248, 118, 100, 101, 227, 109, 208, 110, 147, 162, 435, 213,
            146, 493, 491, 143, 218, 6, 7, 365, 38, 207, 485, 358, 107, 357, 379, 355, 386, 50,
            232, 389, 234, 141, 345, 344, 474, 390, 341, 17, 18, 173, 132, 20, 246, 98, 330, 169,
            460, 328, 457, 182, 324, 455, 405, 92, 63, 259, 127, 23, 64, 197, 317, 408, 89, 315,
            412, 269, 140, 448, 65, 168, 311, 414, 187, 277, 415, 306, 167, 165, 28, 137, 429, 120,
            437, 431, 27, 434, 369, 191, 1, 193, 189, 74, 135, 81, 440, 424, 421, 301, 30, 31, 32,
            444, 195, 416, 278, 445, 447, 274, 86, 272, 88, 450, 265, 264, 406, 262, 452, 90, 145,
            454, 200, 253, 404, 180, 499, 97, 461, 402, 58, 464, 334, 398, 393, 172, 205, 129, 340,
            471, 342, 239, 14, 236, 346, 12, 11, 47, 387, 177, 230, 384, 383, 103, 104, 206, 150,
            359, 378, 222, 130, 39, 44, 221, 43, 40, 41, 111, 78, 42, 45, 373, 374, 494, 376, 215,
            113, 224, 225, 381, 151, 212, 231, 385, 176, 233, 388, 51, 240, 338, 337, 179, 15, 468,
            242, 34, 57, 245, 400, 133, 59, 60, 326, 62, 318, 156, 93, 409, 410, 453, 314, 312,
            260, 196, 87, 305, 275, 303, 302, 446, 166, 85, 280, 67, 441, 68, 134, 284, 80, 190,
            432, 368, 79, 292, 289, 163, 73, 283, 281, 426, 139, 443, 299, 29, 300, 304, 123, 270,
            308, 198, 310, 411, 199, 316, 456, 252, 22, 407, 201, 202, 333, 19, 204, 396, 241, 467,
            395, 394, 470, 52, 155, 473, 238, 154, 102, 477, 153, 235, 350, 353, 382, 380, 8, 356,
            220, 117, 360, 3, 144, 216, 492, 37, 361, 495, 214, 323, 354, 0,
        ];
        assert_eq!(v.iter().map(|x| x.1).collect::<Vec<_>>(), want);
    }
}
