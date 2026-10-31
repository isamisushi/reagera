use reagera::{App, Body, Ctx, Frame, Html, Nothing, Options, Rec, Record, Store};
use std::{hint::black_box, time::Instant};

struct History;
impl App for History {
    const NAME: &'static str = "history-bench/1";
    type Event = u64;
    type State = u64;
    type Effect = Nothing;
    type Outcome = Nothing;
    fn apply(state: &mut u64, event: &u64, _: &mut Ctx<Self>) {
        *state = state.wrapping_mul(31).wrapping_add(*event);
    }
    fn view(_: &u64, _: &Frame) -> Html<u64> {
        reagera::html::div().build()
    }
}

fn empty() -> Store<History> {
    Store::new(Options::default().origin("bench").clock(|| 0))
}

fn main() {
    let n: usize = std::env::var("REAGERA_BENCH_EVENTS").ok()
        .map(|value| value.parse().expect("REAGERA_BENCH_EVENTS must be an integer"))
        .unwrap_or(5000);
    assert!(n > 0 && n <= u32::MAX as usize);
    let records: Vec<Rec<History>> = (0..n).map(|i|
        Record::new(i as i64, "source", i as u32, Body::Event(i as u64))
    ).collect();
    let expected = (0..n).fold(0u64, |state, i| state.wrapping_mul(31).wrapping_add(i as u64));
    println!("events={n}, checkpoint_interval=64; construction and input cloning excluded");
    for reversed in [false, true] {
        for batched in [false, true] {
            let mut input = records.clone();
            if reversed { input.reverse(); }
            let mut store = empty();
            let start = Instant::now();
            if batched {
                assert_eq!(store.receive_batch(input), n);
            } else {
                for record in input { assert!(store.receive(record)); }
            }
            let elapsed = start.elapsed();
            assert_eq!(*store.state(), expected);
            assert!(store.take_rejections().is_empty());
            println!("reverse={reversed}, batch={batched}: {:.3} ms", elapsed.as_secs_f64() * 1000.0);
            black_box(store);
        }
    }
    let mut store = empty();
    store.receive_batch(records);
    let start = Instant::now();
    let bytes = store.export();
    println!("export: {:.3} ms, {} bytes", start.elapsed().as_secs_f64() * 1000.0, bytes.len());
    let start = Instant::now();
    let mut copy = Store::<History>::import(&bytes, Options::default().origin("copy")).unwrap();
    println!("import: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
    assert_eq!(*copy.state(), expected);
    assert_eq!(copy.export(), bytes);
    let start = Instant::now();
    for i in 0..1000 { black_box(copy.state_at((i * n / 1000) as i64)); }
    println!("1000 historical queries: {:.3} ms", start.elapsed().as_secs_f64() * 1000.0);
}
