use nickel_session_protocol::controller_broker::{
    BrokerMessage, ControllerBroker, HostId, IngressDisposition, TransferStatus,
};

const STEPS: usize = 32;
const SEEDS: [u64; 3] = [7, 0x5eed, 0xdead_beef];

fn next(seed: &mut u64) -> u64 {
    *seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
    *seed
}

#[test]
fn bounded_generated_transfer_timeout_and_overflow_sequences_fence_delivery() {
    for mut seed in SEEDS {
        let mut broker = ControllerBroker::new(3);
        let a = broker.attach(HostId(1));
        let mut b = broker.attach(HostId(2));
        let mut lease = broker.grant(HostId(1), a).unwrap();

        for step in 0..STEPS {
            if next(&mut seed) & 1 == 0 {
                let TransferStatus::Pending { cutoff, .. } =
                    broker.begin_transfer(HostId(2), b, step as u64 * 10, 5)
                else {
                    panic!("active lease must enter transfer")
                };
                assert!(matches!(
                    broker.ingest(step),
                    IngressDisposition::RejectedTransfer { .. }
                ));
                if next(&mut seed) & 2 == 0 {
                    let status = broker.acknowledge_quiescence(
                        lease.host,
                        lease.connection_generation,
                        lease.epoch,
                        cutoff,
                    );
                    let TransferStatus::Granted(new_lease) = status else {
                        panic!("neutral quiescent transfer must grant")
                    };
                    lease = new_lease;
                } else {
                    assert_eq!(
                        broker.expire_transfer(step as u64 * 10 + 5),
                        TransferStatus::Failed
                    );
                    assert_eq!(broker.active_lease(), None);
                    assert!(matches!(
                        broker.ingest(step),
                        IngressDisposition::RejectedResetBarrier { .. }
                    ));
                    broker.set_neutral(true);
                    assert!(broker.grant(HostId(1), a).is_none());
                    assert_eq!(
                        broker.acknowledge_verified_termination(
                            lease.host,
                            lease.connection_generation
                        ),
                        TransferStatus::Failed
                    );
                    b = broker.attach(HostId(2));
                    let TransferStatus::Granted(recovered) =
                        broker.begin_transfer(HostId(2), b, step as u64 * 10 + 6, 5)
                    else {
                        panic!("cleared timeout recovery must accept a fresh destination")
                    };
                    lease = recovered;
                }
            } else {
                for payload in 0..3 {
                    assert!(matches!(
                        broker.ingest(payload),
                        IngressDisposition::Delivered { lease: current, .. } if current == lease
                    ));
                }
                assert!(matches!(
                    broker.ingest(4),
                    IngressDisposition::OverflowReset { .. }
                ));
                assert_eq!(broker.active_lease(), None);
                let messages = broker.drain(lease.host, lease.connection_generation);
                assert!(matches!(
                    messages.as_slice(),
                    [BrokerMessage::StreamReset { .. }]
                ));
                broker.set_neutral(true);
                lease = broker.grant(HostId(1), a).unwrap();
            }
            assert_eq!(broker.active_lease(), Some(lease));
        }
    }
}

#[test]
fn exact_a_internal_b_trace_never_exposes_an_unrelated_pending_lease() {
    let mut broker = ControllerBroker::<()>::new(4);
    let internal = broker.attach(HostId(0));
    let a = broker.attach(HostId(1));
    let b = broker.attach(HostId(2));
    let a_lease = broker.grant(HostId(1), a).unwrap();
    broker.set_neutral(false);

    let internal_status = broker.begin_transfer(HostId(0), internal, 0, 10);
    let TransferStatus::Pending { cutoff, .. } = internal_status else {
        panic!("A to internal must enter revocation");
    };
    assert_eq!(
        broker.begin_transfer(HostId(2), b, 1, 10),
        TransferStatus::Failed
    );
    assert!(matches!(
        broker.ingest(()),
        IngressDisposition::RejectedTransfer { .. }
    ));
    assert_eq!(
        broker.acknowledge_quiescence(HostId(1), a, a_lease.epoch, cutoff),
        internal_status
    );
    let internal_lease = broker.set_neutral(true).unwrap();
    assert_eq!(internal_lease.host, HostId(0));

    let TransferStatus::Pending {
        cutoff: internal_cutoff,
        ..
    } = broker.begin_transfer(HostId(2), b, 2, 10)
    else {
        panic!("B retry must revoke the internal successor");
    };
    let TransferStatus::Granted(b_lease) =
        broker.acknowledge_quiescence(HostId(0), internal, internal_lease.epoch, internal_cutoff)
    else {
        panic!("neutral internal quiescence must grant B");
    };
    assert_eq!(b_lease.host, HostId(2));
    assert_eq!(b_lease.connection_generation, b);
}
