// Finite native allocation/ownership controls, compiled alongside unchanged
// declarations and methods. These are not the universal Lean frame theorem.
fn ticket(n: usize) -> Ticket {
    Ticket { newest: n, anchor: n + 1, watermark: n + 2,
        axes: vec![TicketAxis { domain: n + 3, end: n + 4, coordinate: n + 5 }],
        lower_bound: n + 6, turn: n + 7, phase: TicketPhase::Carry, scan: n + 8, carry: n + 9 }
}

fn ticket_state(t: &Ticket) -> (usize, usize, usize, *const TicketAxis, Vec<TicketAxis>,
                               usize, usize, TicketPhase, usize, usize) {
    (t.newest, t.anchor, t.watermark, t.axes.as_ptr(), t.axes.clone(),
     t.lower_bound, t.turn, t.phase, t.scan, t.carry)
}

fn main() {
    let child = Arc::new(Expression { owner: Arc::new(()), body: Body::Leaf(7),
        summary: Summary { outputs: Arc::from([13u64]), dependencies: 17 }, size: 19 });
    for phase in [ConstructorPhase::Merge, ConstructorPhase::Freeze, ConstructorPhase::Anchor,
                  ConstructorPhase::Bounds, ConstructorPhase::Publish, ConstructorPhase::Idle] {
        for sleeping in [false, true] {
            let mut c = Constructor { alternative: 23,
                axes: vec![Axis { domain: 29, consumed: 31, before: 37, after: 41 }],
                phase, scan: 47, newest: Some(53), anchor: 59, watermark: 67, observed: 71,
                building: Some(ticket(101)), tickets: vec![ticket(201), ticket(301)],
                heap: HeapPhase::Swap(2, 3), active: Some(ticket(401)), next_turn: 503,
                service: true, sleeping, ticket_cells: 509,
                tuple: Some(Tuple { alternative: 73, children: vec![Arc::clone(&child)],
                    outputs: vec![79], values: vec![83], phase: TuplePhase::Values, at: 89,
                    case: 97, reset_from: 101, size: 103, dependencies: 107, valid: 109,
                    possible: 113, zero_value: true }) };
            let axes = (c.axes.as_ptr(), c.axes.clone());
            let tickets = c.tickets.as_ptr();
            let retained = [ticket_state(c.building.as_ref().unwrap()), ticket_state(&c.tickets[0]),
                            ticket_state(&c.tickets[1]), ticket_state(c.active.as_ref().unwrap())];
            let t = c.tuple.as_ref().unwrap();
            let pointers = (t.children.as_ptr(), t.outputs.as_ptr(), t.values.as_ptr());
            assert_eq!(c.idle(), sleeping);
            for indexed in [0, 1, 47, usize::MAX] {
                c.merge(indexed);
                assert!(!c.idle());
                assert!(c.phase == ConstructorPhase::Merge && c.scan == 0 && c.newest.is_none());
                assert_eq!(c.watermark, indexed);
                assert_eq!((c.alternative, c.anchor, c.observed, c.next_turn, c.service, c.ticket_cells),
                           (23, 59, 71, 503, true, 509));
                assert_eq!((c.axes.as_ptr(), c.axes.clone()), axes);
                assert_eq!(c.tickets.as_ptr(), tickets);
                assert_eq!(c.tickets.len(), 2);
                assert_eq!(c.heap, HeapPhase::Swap(2, 3));
                assert_eq!([ticket_state(c.building.as_ref().unwrap()), ticket_state(&c.tickets[0]),
                            ticket_state(&c.tickets[1]), ticket_state(c.active.as_ref().unwrap())], retained);
                let t = c.tuple.as_ref().unwrap();
                assert_eq!((t.children.as_ptr(), t.outputs.as_ptr(), t.values.as_ptr()), pointers);
                assert_eq!((t.alternative, t.at, t.case, t.reset_from, t.size), (73, 89, 97, 101, 103));
                assert_eq!((t.dependencies, t.valid, t.possible, t.zero_value), (107, 109, 113, true));
                assert_eq!(t.phase, TuplePhase::Values);
                assert_eq!(t.outputs, [79]); assert_eq!(t.values, [83]);
                assert_eq!(t.children.len(), 1);
                assert!(Arc::ptr_eq(&t.children[0], &child));
                assert_eq!(Arc::strong_count(&child), 2);
                assert_eq!(Arc::strong_count(&child.owner), 1);
            }
        }
    }
}
