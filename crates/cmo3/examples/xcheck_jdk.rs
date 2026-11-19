use std::io::BufRead;
fn main() {
    let dir = std::env::args().nth(1).unwrap();
    let mut bad = 0;
    let mut n = 0;
    for line in
        std::io::BufReader::new(std::fs::File::open(format!("{dir}/floats.txt")).unwrap()).lines()
    {
        let line = line.unwrap();
        let (bits, want) = line.split_once(' ').unwrap();
        let v = f32::from_bits(bits.parse::<i32>().unwrap() as u32);
        let got = cmo3::numfmt::float_to_string(v);
        n += 1;
        if got != want {
            bad += 1;
            if bad <= 10 {
                println!("f32 {bits}: java={want} rust={got}");
            }
        }
    }
    println!("f32: {n} checked, {bad} mismatches");
    let mut bad = 0;
    let mut n = 0;
    for line in
        std::io::BufReader::new(std::fs::File::open(format!("{dir}/doubles.txt")).unwrap()).lines()
    {
        let line = line.unwrap();
        let (bits, want) = line.split_once(' ').unwrap();
        let v = f64::from_bits(bits.parse::<i64>().unwrap() as u64);
        let got = cmo3::numfmt::double_to_string(v);
        n += 1;
        if got != want {
            bad += 1;
            if bad <= 10 {
                println!("f64 {bits}: java={want} rust={got}");
            }
        }
    }
    println!("f64: {n} checked, {bad} mismatches");
}
