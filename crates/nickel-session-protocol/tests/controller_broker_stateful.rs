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
        let b = broker.attach(HostId(2));
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
                    lease = broker.grant(HostId(1), a).unwrap();
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
