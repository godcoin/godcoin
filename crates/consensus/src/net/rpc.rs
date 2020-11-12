use super::{super::log::Entry, Serializable};
use bytes::{BufMut, BytesMut};
use godcoin::serializer::BufRead;
use std::io::{self, Cursor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    PreVote(PreVoteReq),
    RequestVote(RequestVoteReq),
    AppendEntries(AppendEntriesReq),
    LogSync(LogSyncReq),
}

impl Serializable<Self> for Request {
    fn serialize(&self, dst: &mut BytesMut) {
        match self {
            Self::PreVote(req) => {
                dst.put_u8(0x01);
                dst.put_u64(req.last_index);
                dst.put_u64(req.last_term);
            }
            Self::RequestVote(req) => {
                dst.put_u8(0x02);
                dst.put_u64(req.term);
                dst.put_u64(req.last_index);
                dst.put_u64(req.last_term);
            }
            Self::AppendEntries(req) => {
                dst.put_u8(0x03);
                dst.put_u64(req.term);
                dst.put_u64(req.prev_index);
                dst.put_u64(req.prev_term);
                dst.put_u64(req.leader_commit);
                dst.put_u64(req.entries.len() as u64);
                for e in &req.entries {
                    e.serialize(dst);
                }
            }
            Self::LogSync(req) => {
                dst.put_u8(0x04);
                dst.put_u64(req.last_index);
                dst.put_u64(req.last_term);
            }
        }
    }

    fn byte_size(&self) -> usize {
        let size = match self {
            Self::PreVote(_) => 16,
            Self::RequestVote(_) => 24,
            Self::AppendEntries(req) => {
                let entry_len = req.entries.iter().fold(0, |mut acc, entry| {
                    acc += entry.byte_size();
                    acc
                });
                40 + entry_len
            }
            Self::LogSync(_) => 16,
        };
        // Add 1 byte for the tag type
        size + 1
    }

    fn deserialize(src: &mut Cursor<&[u8]>) -> io::Result<Self> {
        let tag = src.take_u8()?;
        match tag {
            0x01 => {
                let last_index = src.take_u64()?;
                let last_term = src.take_u64()?;
                Ok(Self::PreVote(PreVoteReq {
                    last_index,
                    last_term,
                }))
            }
            0x02 => {
                let term = src.take_u64()?;
                let last_index = src.take_u64()?;
                let last_term = src.take_u64()?;
                Ok(Self::RequestVote(RequestVoteReq {
                    term,
                    last_index,
                    last_term,
                }))
            }
            0x03 => {
                let term = src.take_u64()?;
                let prev_index = src.take_u64()?;
                let prev_term = src.take_u64()?;
                let leader_commit = src.take_u64()?;
                let entries = {
                    let len = src.take_u64()?;
                    let mut entries = Vec::with_capacity(len as usize);
                    for _ in 0..len {
                        entries.push(Entry::deserialize(src)?);
                    }
                    entries
                };
                Ok(Self::AppendEntries(AppendEntriesReq {
                    term,
                    prev_index,
                    prev_term,
                    leader_commit,
                    entries,
                }))
            }
            0x04 => {
                let last_index = src.take_u64()?;
                let last_term = src.take_u64()?;
                Ok(Self::LogSync(LogSyncReq {
                    last_index,
                    last_term,
                }))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid tag type on Request",
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreVoteReq {
    pub last_index: u64,
    pub last_term: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestVoteReq {
    /// Term of the candidate
    pub term: u64,
    pub last_index: u64,
    pub last_term: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendEntriesReq {
    /// Current term of the leader
    pub term: u64,
    /// Log index preceding new entries
    pub prev_index: u64,
    /// The term of the previous index
    pub prev_term: u64,
    /// The latest stable entry in the log
    pub leader_commit: u64,
    /// Entries that should be committed (or empty for a heartbeat)
    pub entries: Vec<Entry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSyncReq {
    pub last_index: u64,
    pub last_term: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    PreVote(PreVoteRes),
    RequestVote(RequestVoteRes),
    AppendEntries(AppendEntriesRes),
    LogSync(LogSyncRes),
}

impl Serializable<Self> for Response {
    fn serialize(&self, dst: &mut BytesMut) {
        match self {
            Self::PreVote(res) => {
                dst.put_u8(0x01);
                dst.put_u8(res.approved.into());
            }
            Self::RequestVote(res) => {
                dst.put_u8(0x02);
                dst.put_u64(res.current_term);
                dst.put_u8(res.approved.into());
            }
            Self::AppendEntries(res) => {
                dst.put_u8(0x03);
                dst.put_u64(res.current_term);
                dst.put_u8(res.success.into());
                dst.put_u64(res.index);
            }
            Self::LogSync(res) => {
                dst.put_u8(0x04);
                dst.put_u64(res.leader_commit);
                dst.put_u8(res.complete.into());
                dst.put_u64(res.entries.len() as u64);
                for e in &res.entries {
                    e.serialize(dst);
                }
            }
        }
    }

    fn byte_size(&self) -> usize {
        let size = match self {
            Self::PreVote(_) => 1,
            Self::RequestVote(_) => 9,
            Self::AppendEntries(_) => 17,
            Self::LogSync(res) => {
                let entry_len = res.entries.iter().fold(0, |mut acc, entry| {
                    acc += entry.byte_size();
                    acc
                });
                17 + entry_len
            }
        };
        // Add 1 byte for the tag type
        size + 1
    }

    fn deserialize(src: &mut Cursor<&[u8]>) -> io::Result<Self> {
        let tag = src.take_u8()?;
        match tag {
            0x01 => {
                let approved = src.take_u8()? != 0;
                Ok(Self::PreVote(PreVoteRes { approved }))
            }
            0x02 => {
                let current_term = src.take_u64()?;
                let approved = src.take_u8()? != 0;
                Ok(Self::RequestVote(RequestVoteRes {
                    current_term,
                    approved,
                }))
            }
            0x03 => {
                let current_term = src.take_u64()?;
                let success = src.take_u8()? != 0;
                let index = src.take_u64()?;
                Ok(Self::AppendEntries(AppendEntriesRes {
                    current_term,
                    success,
                    index,
                }))
            }
            0x04 => {
                let leader_commit = src.take_u64()?;
                let complete = src.take_u8()? != 0;
                let entries = {
                    let len = src.take_u64()?;
                    let mut entries = Vec::with_capacity(len as usize);
                    for _ in 0..len {
                        entries.push(Entry::deserialize(src)?);
                    }
                    entries
                };
                Ok(Self::LogSync(LogSyncRes {
                    leader_commit,
                    complete,
                    entries,
                }))
            }
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid tag type on Request",
            )),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreVoteRes {
    pub approved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestVoteRes {
    /// Term of the current node
    pub current_term: u64,
    /// Whether the current node approves the other node becoming a leader
    pub approved: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppendEntriesRes {
    /// Term of the current node
    pub current_term: u64,
    /// Whether the entries were successfully committed
    pub success: bool,
    /// Last acknowledged index, only on success
    pub index: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogSyncRes {
    /// The latest stable entry in the log
    pub leader_commit: u64,
    /// Tells the client whether or not the log sync is finished
    pub complete: bool,
    /// Entries that should be committed
    pub entries: Vec<Entry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytes::Bytes;

    #[test]
    fn serialize_prevote_req() {
        test_req_serialization(Request::PreVote(PreVoteReq {
            last_index: 10,
            last_term: 20,
        }));
    }

    #[test]
    fn serialize_request_vote_req() {
        test_req_serialization(Request::RequestVote(RequestVoteReq {
            term: 1234,
            last_index: 10,
            last_term: 20,
        }));
    }

    #[test]
    fn serialize_append_entries_req() {
        test_req_serialization(Request::AppendEntries(AppendEntriesReq {
            term: 1234,
            prev_index: 100,
            prev_term: 7,
            leader_commit: 95,
            entries: vec![],
        }));

        test_req_serialization(Request::AppendEntries(AppendEntriesReq {
            term: 1234,
            prev_index: 100,
            prev_term: 7,
            leader_commit: 95,
            entries: {
                let cap = 25;
                let mut entries = Vec::with_capacity(cap);
                for i in 1..=cap {
                    entries.push(Entry {
                        index: i as u64,
                        term: 12345,
                        data: Bytes::copy_from_slice(&[1, 2, 3]),
                    });
                }
                entries
            },
        }));
    }

    #[test]
    fn serialize_log_sync_req() {
        test_req_serialization(Request::LogSync(LogSyncReq {
            last_index: 1234,
            last_term: 456,
        }));
    }

    #[test]
    fn serialize_prevote_res() {
        test_res_serialization(Response::PreVote(PreVoteRes { approved: true }));
        test_res_serialization(Response::PreVote(PreVoteRes { approved: false }));
    }

    #[test]
    fn serialize_request_vote_res() {
        test_res_serialization(Response::RequestVote(RequestVoteRes {
            current_term: 1234,
            approved: true,
        }));

        test_res_serialization(Response::RequestVote(RequestVoteRes {
            current_term: 1234,
            approved: false,
        }));
    }

    #[test]
    fn serialize_append_entries_res() {
        test_res_serialization(Response::AppendEntries(AppendEntriesRes {
            current_term: 1234,
            success: true,
            index: 0,
        }));

        test_res_serialization(Response::AppendEntries(AppendEntriesRes {
            current_term: 1234,
            success: false,
            index: 1234,
        }));
    }

    #[test]
    fn serialize_log_sync_res() {
        test_res_serialization(Response::LogSync(LogSyncRes {
            leader_commit: 10,
            complete: true,
            entries: vec![],
        }));

        test_res_serialization(Response::LogSync(LogSyncRes {
            leader_commit: 95,
            complete: false,
            entries: {
                let cap = 25;
                let mut entries = Vec::with_capacity(cap);
                for i in 1..=cap {
                    entries.push(Entry {
                        index: i as u64,
                        term: 12345,
                        data: Bytes::copy_from_slice(&[1, 2, 3]),
                    });
                }
                entries
            },
        }));
    }

    fn test_req_serialization(req_a: Request) {
        let mut bytes = BytesMut::with_capacity(req_a.byte_size());
        req_a.serialize(&mut bytes);
        verify_byte_len(&bytes, req_a.byte_size());

        let req_b = Request::deserialize(&mut Cursor::new(bytes.as_ref())).unwrap();
        assert_eq!(req_a, req_b);
    }

    fn test_res_serialization(res_a: Response) {
        let mut bytes = BytesMut::with_capacity(res_a.byte_size());
        res_a.serialize(&mut bytes);
        verify_byte_len(&bytes, res_a.byte_size());

        let res_b = Response::deserialize(&mut Cursor::new(bytes.as_ref())).unwrap();
        assert_eq!(res_a, res_b);
    }

    #[track_caller]
    fn verify_byte_len(bytes: &BytesMut, expected_size: usize) {
        assert_eq!(bytes.len(), expected_size);
        assert_eq!(bytes.capacity(), expected_size);
    }
}