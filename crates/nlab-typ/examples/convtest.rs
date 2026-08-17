fn main() {
    for tex in ["{a} \\atop {b}", "\\phantom{WIDE}", "\\textbf{Cat}", "\\shortmid", "\\scalebox{.6}{x}"] {
        println!("{:?} => {:?}", tex, mitex::convert_math_itex(tex, None));
    }
}
