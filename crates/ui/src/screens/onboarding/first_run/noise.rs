#[derive(Clone, Copy, Default)]
struct Contribution {
    delta: [f64; 3],
    vertex: [i32; 3],
}

impl Contribution {
    fn new(raw: &[i8]) -> Self {
        let squish = f64::from(raw[0]) * (1. / 3.);
        let vertex = [i32::from(raw[1]), i32::from(raw[2]), i32::from(raw[3])];
        Self {
            delta: vertex.map(|coordinate| -f64::from(coordinate) - squish),
            vertex,
        }
    }
}

pub(super) struct Noise3d {
    permutation: [u8; 256],
    gradients: [u8; 256],
    contributions: [[Contribution; 8]; 24],
    lengths: [usize; 24],
}

impl Noise3d {
    pub(super) fn new(mut seed: u32) -> Self {
        let mut result = Self {
            permutation: [0; 256],
            gradients: [0; 256],
            contributions: [[Contribution::default(); 8]; 24],
            lengths: [0; 24],
        };
        let advance = |seed: u32| seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        seed = advance(advance(advance(seed)));
        let mut source: [u8; 256] = std::array::from_fn(|index| index as u8);
        for index in (0..256).rev() {
            seed = advance(seed);
            let selected = ((u64::from(seed) + 31) % (index as u64 + 1)) as usize;
            result.permutation[index] = source[selected];
            result.gradients[index] = result.permutation[index] % 24 * 3;
            source[selected] = source[index];
        }
        for (group, raw) in RAW_CONTRIBUTIONS.as_chunks::<9>().0.iter().enumerate() {
            let mut count = 0;
            for point in BASE[raw[0] as usize]
                .as_chunks::<4>()
                .0
                .iter()
                .chain(raw[1..].as_chunks::<4>().0.iter())
            {
                result.contributions[group][count] = Contribution::new(point);
                count += 1;
            }
            result.lengths[group] = count;
        }
        result
    }

    pub(super) fn sample(&self, x: f64, y: f64, z: f64) -> f64 {
        let stretch = (x + y + z) * (-1. / 6.);
        let stretched = [x + stretch, y + stretch, z + stretch];
        let base = stretched.map(|coordinate| coordinate.floor() as i32);
        let squish = f64::from(base[0] + base[1] + base[2]) * (1. / 3.);
        let delta = [
            x - (f64::from(base[0]) + squish),
            y - (f64::from(base[1]) + squish),
            z - (f64::from(base[2]) + squish),
        ];
        let inside = [
            stretched[0] - f64::from(base[0]),
            stretched[1] - f64::from(base[1]),
            stretched[2] - f64::from(base[2]),
        ];
        let sum = inside[0] + inside[1] + inside[2];
        let hash = ((inside[1] - inside[2] + 1.) as usize)
            | (((inside[0] - inside[1] + 1.) as usize) << 1)
            | (((inside[0] - inside[2] + 1.) as usize) << 2)
            | ((sum as usize) << 3)
            | (((sum + inside[2]) as usize) << 5)
            | (((sum + inside[1]) as usize) << 7)
            | (((sum + inside[0]) as usize) << 9);
        let group = LOOKUP[hash];
        if group == u8::MAX {
            return 0.;
        }
        let group = usize::from(group);
        let mut value = 0.;
        for contribution in &self.contributions[group][..self.lengths[group]] {
            let dx = delta[0] + contribution.delta[0];
            let dy = delta[1] + contribution.delta[1];
            let dz = delta[2] + contribution.delta[2];
            let attenuation = 2. - dx * dx - dy * dy - dz * dz;
            if attenuation > 0. {
                let x = ((base[0] + contribution.vertex[0]) & 255) as usize;
                let y = ((base[1] + contribution.vertex[1]) & 255) as usize;
                let z = ((base[2] + contribution.vertex[2]) & 255) as usize;
                let xy = (usize::from(self.permutation[x]) + y) & 255;
                let xyz = (usize::from(self.permutation[xy]) + z) & 255;
                let gradient = usize::from(self.gradients[xyz]);
                let dot = f64::from(GRADIENTS[gradient]) * dx
                    + f64::from(GRADIENTS[gradient + 1]) * dy
                    + f64::from(GRADIENTS[gradient + 2]) * dz;
                value += attenuation * attenuation * attenuation * attenuation * dot;
            }
        }
        value * (1. / 103.)
    }
}

const fn lookup() -> [u8; 2048] {
    let mut result = [u8::MAX; 2048];
    let mut index = 0;
    while index < LOOKUP_PAIRS.len() {
        result[LOOKUP_PAIRS[index]] = LOOKUP_PAIRS[index + 1] as u8;
        index += 2;
    }
    result
}
const LOOKUP: [u8; 2048] = lookup();

const BASE: &[&[i8]] = &[
    &[0, 0, 0, 0, 1, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1],
    &[2, 1, 1, 0, 2, 1, 0, 1, 2, 0, 1, 1, 3, 1, 1, 1],
    &[
        1, 1, 0, 0, 1, 0, 1, 0, 1, 0, 0, 1, 2, 1, 1, 0, 2, 1, 0, 1, 2, 0, 1, 1,
    ],
];
const GRADIENTS: [i8; 72] = [
    -11, 4, 4, -4, 11, 4, -4, 4, 11, 11, 4, 4, 4, 11, 4, 4, 4, 11, -11, -4, 4, -4, -11, 4, -4, -4,
    11, 11, -4, 4, 4, -11, 4, 4, -4, 11, -11, 4, -4, -4, 11, -4, -4, 4, -11, 11, 4, -4, 4, 11, -4,
    4, 4, -11, -11, -4, -4, -4, -11, -4, -4, -4, -11, 11, -4, -4, 4, -11, -4, 4, -4, -11,
];
const LOOKUP_PAIRS: [usize; 144] = [
    0, 2, 1, 1, 2, 2, 5, 1, 6, 0, 7, 0, 32, 2, 34, 2, 129, 1, 133, 1, 160, 5, 161, 5, 518, 0, 519,
    0, 546, 4, 550, 4, 645, 3, 647, 3, 672, 5, 673, 5, 674, 4, 677, 3, 678, 4, 679, 3, 680, 13,
    681, 13, 682, 12, 685, 14, 686, 12, 687, 14, 712, 20, 714, 18, 809, 21, 813, 23, 840, 20, 841,
    21, 1198, 19, 1199, 22, 1226, 18, 1230, 19, 1325, 23, 1327, 22, 1352, 15, 1353, 17, 1354, 15,
    1357, 17, 1358, 16, 1359, 16, 1360, 11, 1361, 10, 1362, 11, 1365, 10, 1366, 9, 1367, 9, 1392,
    11, 1394, 11, 1489, 10, 1493, 10, 1520, 8, 1521, 8, 1878, 9, 1879, 9, 1906, 7, 1910, 7, 2005,
    6, 2007, 6, 2032, 8, 2033, 8, 2034, 7, 2037, 6, 2038, 7, 2039, 6,
];
const RAW_CONTRIBUTIONS: [i8; 216] = [
    0, 0, 1, -1, 0, 0, 1, 0, -1, 0, 0, -1, 1, 0, 0, 0, 1, -1, 0, 0, -1, 0, 1, 0, 0, -1, 1, 0, 2, 1,
    1, 0, 1, 1, 1, -1, 0, 2, 1, 0, 1, 1, 1, -1, 1, 0, 2, 0, 1, 1, 1, -1, 1, 1, 1, 3, 2, 1, 0, 3, 1,
    2, 0, 1, 3, 2, 0, 1, 3, 1, 0, 2, 1, 3, 0, 2, 1, 3, 0, 1, 2, 1, 1, 1, 0, 0, 2, 2, 0, 0, 1, 1, 0,
    1, 0, 2, 0, 2, 0, 1, 1, 0, 0, 1, 2, 0, 0, 2, 2, 0, 0, 0, 0, 1, 1, -1, 1, 2, 0, 0, 0, 0, 1, -1,
    1, 1, 2, 0, 0, 0, 0, 1, 1, 1, -1, 2, 3, 1, 1, 1, 2, 0, 0, 2, 2, 3, 1, 1, 1, 2, 2, 0, 0, 2, 3,
    1, 1, 1, 2, 0, 2, 0, 2, 1, 1, -1, 1, 2, 0, 0, 2, 2, 1, 1, -1, 1, 2, 2, 0, 0, 2, 1, -1, 1, 1, 2,
    0, 0, 2, 2, 1, -1, 1, 1, 2, 0, 2, 0, 2, 1, 1, 1, -1, 2, 2, 0, 0, 2, 1, 1, 1, -1, 2, 0, 2, 0,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn original_seed_and_coordinate_vectors_match() {
        for (seed, point, expected) in [
            (
                1_792_083_600_000_u64,
                [0.15, 0.15, 0.03],
                0.3361360892877665,
            ),
            (
                1_792_083_600_000_u64,
                [19.35, 14.85, 2.22],
                0.041264697150697485,
            ),
            (4_294_967_295_u64, [-0.3, 2.7, 1.8], -0.04647065708737889),
        ] {
            let result = Noise3d::new(seed as u32).sample(point[0], point[1], point[2]);
            assert!((result - expected).abs() < 1e-12, "{result} != {expected}");
        }
    }
}
