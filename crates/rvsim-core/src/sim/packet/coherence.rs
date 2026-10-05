//! Messages between requesting agents (private L2s) and the home agent.
//!
//! Names follow CHI: a requester asks for a line (`ReadShared`,
//! `ReadUnique`), for permission to write a line it already holds
//! (`CleanUnique`), or tells the home it gave a line up (`WriteBack`,
//! `Evict`); the home snoops other holders (`SnpShared`, `SnpUnique`,
//! `SnpInvalid`), they answer (`SnoopResp`), and the home completes the
//! requester (`CompData`, `Comp`). Each message belongs to one class so an
//! interconnect can give every class its own virtual channel.

use crate::common::{CoreId, LineAddr};
use crate::sim::components::ReqId;
use crate::sim::packet::{Maintenance, MesiState};

/// An endpoint of the coherence interconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Node {
    /// A core's private hierarchy (its L2 is the requesting agent).
    Core(CoreId),
    /// The home agent.
    Home,
}

/// Virtual-channel class; every message is exactly one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MsgClass {
    /// Requester → home.
    Request,
    /// Home → holder.
    Snoop,
    /// Holder → home, and home → requester completions without data.
    Response,
    /// Messages carrying a line: completions with data, and a holder's
    /// modified copy answering a snoop (CHI's `SnpRespData`).
    Data,
}

impl MsgClass {
    /// Every class, in virtual-channel order.
    pub const ALL: [Self; 4] = [Self::Request, Self::Snoop, Self::Response, Self::Data];

    /// Position in [`MsgClass::ALL`].
    #[must_use]
    pub const fn index(self) -> usize {
        match self {
            Self::Request => 0,
            Self::Snoop => 1,
            Self::Response => 2,
            Self::Data => 3,
        }
    }
}

/// What a requester asks the home agent for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReqKind {
    /// A copy to read: the requester ends in Shared or Exclusive.
    ReadShared,
    /// A copy to write: every other holder is invalidated, the requester
    /// ends in Modified.
    ReadUnique,
    /// Permission to write a line the requester holds Shared.
    CleanUnique,
    /// The requester dropped the line; `dirty` says the home must
    /// record its data as the current copy.
    WriteBack {
        /// Whether the line was modified.
        dirty: bool,
    },
    /// The requester silently dropped a clean line.
    Evict,
    /// A cache-maintenance operation on the line, applied to every other
    /// holder and then to the LLC and memory (CHI's `CleanShared`,
    /// `CleanInvalid`, `MakeInvalid`). The requester has already applied it
    /// to its own copy.
    Maintain {
        /// What to do to the line.
        op: Maintenance,
        /// The requester's copy was dirty; its data travels with the
        /// request.
        dirty: bool,
    },
}

/// What the home asks of a holder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnoopKind {
    /// Keep at most a Shared copy.
    Shared,
    /// Drop the line for a writer.
    Unique,
    /// Drop the line because the home no longer tracks it.
    Invalid,
    /// Keep the line, clean: dirty data goes back to the home.
    Clean,
    /// Drop the line, discarding dirty data.
    MakeInvalid,
}

/// One message on the coherence interconnect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoherenceMsg {
    /// Requester → home.
    Req {
        /// The requester's correlator (its MSHR or writeback entry).
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// What is asked.
        kind: ReqKind,
        /// Who asks.
        requester: CoreId,
    },
    /// Home → holder.
    Snoop {
        /// The home's transaction.
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// What the holder must do.
        kind: SnoopKind,
        /// Who is snooped.
        target: CoreId,
    },
    /// Holder → home.
    SnoopResp {
        /// The home's transaction.
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// Who answers.
        from: CoreId,
        /// Whether the holder had any copy before the snoop.
        had_copy: bool,
        /// Whether that copy was modified (the holder's data is the
        /// current copy of the line).
        dirty: bool,
    },
    /// Home → requester, with the line.
    CompData {
        /// The requester's correlator.
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// Who is completed.
        to: CoreId,
        /// State the requester may install.
        state: MesiState,
    },
    /// Home → requester, without data (a permission grant or an ack).
    Comp {
        /// The requester's correlator.
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// Who is completed.
        to: CoreId,
        /// State the requester may install (`Modified` after a
        /// `CleanUnique`, `Invalid` for a writeback ack).
        state: MesiState,
    },
    /// Requester → home: the completion was taken up, so the home may start
    /// the next transaction on the line (a later snoop cannot overtake the
    /// completion).
    CompAck {
        /// The requester's correlator.
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// Who acknowledges.
        from: CoreId,
    },
    /// Requester → home: an access that takes no part in coherence (an
    /// uncached access from a core without caches), carried to memory
    /// without snooping. The request itself waits at the fabric.
    NoSnp {
        /// The requester's correlator.
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// Who asks.
        requester: CoreId,
        /// Payload bytes carried with the request (a write's data).
        bytes: usize,
    },
    /// Home → requester: the answer to a [`CoherenceMsg::NoSnp`].
    NoSnpData {
        /// The requester's correlator.
        txn: ReqId,
        /// Line concerned.
        line: LineAddr,
        /// Who is answered.
        to: CoreId,
        /// Payload bytes carried back.
        bytes: usize,
    },
}

impl CoherenceMsg {
    /// Virtual channel the message travels on.
    #[must_use]
    pub const fn class(self) -> MsgClass {
        match self {
            Self::Req { .. } | Self::NoSnp { .. } => MsgClass::Request,
            Self::Snoop { .. } => MsgClass::Snoop,
            Self::SnoopResp { dirty: false, .. } | Self::Comp { .. } | Self::CompAck { .. } => {
                MsgClass::Response
            }
            Self::SnoopResp { dirty: true, .. }
            | Self::CompData { .. }
            | Self::NoSnpData { .. } => MsgClass::Data,
        }
    }

    /// Where the message goes.
    #[must_use]
    pub const fn destination(self) -> Node {
        match self {
            Self::Req { .. }
            | Self::SnoopResp { .. }
            | Self::CompAck { .. }
            | Self::NoSnp { .. } => Node::Home,
            Self::Snoop { target, .. } => Node::Core(target),
            Self::CompData { to, .. } | Self::Comp { to, .. } | Self::NoSnpData { to, .. } => {
                Node::Core(to)
            }
        }
    }

    /// Bytes on the wire: a header, plus the line for every message that
    /// carries one: a fill, and a modified copy written back, flushed or
    /// handed over in answer to a snoop.
    #[must_use]
    pub const fn bytes(self, line_bytes: usize) -> usize {
        const HEADER: usize = 8;
        match self {
            Self::CompData { .. }
            | Self::SnoopResp { dirty: true, .. }
            | Self::Req { kind: ReqKind::WriteBack { dirty: true }, .. }
            | Self::Req { kind: ReqKind::Maintain { dirty: true, .. }, .. } => HEADER + line_bytes,
            Self::NoSnp { bytes, .. } | Self::NoSnpData { bytes, .. } => HEADER + bytes,
            _ => HEADER,
        }
    }

    #[cfg(test)]
    /// Transaction correlator.
    #[must_use]
    pub const fn txn(self) -> ReqId {
        match self {
            Self::Req { txn, .. }
            | Self::Snoop { txn, .. }
            | Self::SnoopResp { txn, .. }
            | Self::CompData { txn, .. }
            | Self::Comp { txn, .. }
            | Self::CompAck { txn, .. }
            | Self::NoSnp { txn, .. }
            | Self::NoSnpData { txn, .. } => txn,
        }
    }

    /// Line concerned.
    #[must_use]
    pub const fn line(self) -> LineAddr {
        match self {
            Self::Req { line, .. }
            | Self::Snoop { line, .. }
            | Self::SnoopResp { line, .. }
            | Self::CompData { line, .. }
            | Self::Comp { line, .. }
            | Self::CompAck { line, .. }
            | Self::NoSnp { line, .. }
            | Self::NoSnpData { line, .. } => line,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::PhysAddr;

    #[test]
    fn classes_destinations_and_sizes() {
        let line = LineAddr::from_phys(PhysAddr::new(0x1000), 64);
        let req = CoherenceMsg::Req {
            txn: ReqId::new(1),
            line,
            kind: ReqKind::ReadShared,
            requester: CoreId::new(2),
        };
        assert_eq!(req.class(), MsgClass::Request);
        assert_eq!(req.destination(), Node::Home);
        assert_eq!(req.bytes(64), 8);
        let data = CoherenceMsg::CompData {
            txn: ReqId::new(1),
            line,
            to: CoreId::new(2),
            state: MesiState::Shared,
        };
        assert_eq!(data.class(), MsgClass::Data);
        assert_eq!(data.destination(), Node::Core(CoreId::new(2)));
        assert_eq!(data.bytes(64), 72);
        let snoop = CoherenceMsg::Snoop {
            txn: ReqId::new(3),
            line,
            kind: SnoopKind::Unique,
            target: CoreId::new(0),
        };
        assert_eq!(snoop.class(), MsgClass::Snoop);
        assert_eq!(snoop.destination(), Node::Core(CoreId::new(0)));
    }

    #[test]
    fn a_modified_copy_travels_with_its_line() {
        let line = LineAddr::from_phys(PhysAddr::new(0x1000), 64);
        let (txn, core) = (ReqId::new(1), CoreId::new(0));
        let writeback = |dirty| CoherenceMsg::Req {
            txn,
            line,
            kind: ReqKind::WriteBack { dirty },
            requester: core,
        };
        let answer =
            |dirty| CoherenceMsg::SnoopResp { txn, line, from: core, had_copy: true, dirty };

        assert_eq!((writeback(true).bytes(64), writeback(false).bytes(64)), (72, 8));
        assert_eq!((answer(true).bytes(64), answer(false).bytes(64)), (72, 8));
        assert_eq!(
            (answer(true).class(), answer(false).class()),
            (MsgClass::Data, MsgClass::Response)
        );
        assert_eq!(writeback(true).class(), MsgClass::Request);
    }
}
