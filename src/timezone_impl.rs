use chrono::{Offset, TimeZone, NaiveDate, NaiveDateTime, LocalResult, Duration};
use std::fmt::{Debug, Display, Formatter, Error};
use std::cmp::Ordering;
use binary_search::binary_search;
use super::timezones::Tz;

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct FixedTimespan {
    pub utc_offset: i64,
    pub dst_offset: i64,
    pub name: &'static str,
}

impl Offset for FixedTimespan {
    fn local_minus_utc(&self) -> Duration {
        Duration::seconds(self.utc_offset + self.dst_offset)
    }
}

impl Display for FixedTimespan {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        write!(f, "{}", self.name)
    }
}

impl Debug for FixedTimespan {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        write!(f, "{}", self.name)
    }
}

#[derive(Copy, Clone, PartialEq, Eq)]
pub struct TzOffset {
    tz: Tz,
    offset: FixedTimespan,
}

impl TzOffset {
    fn new(tz: Tz, offset: FixedTimespan) -> Self {
        TzOffset {
            tz: tz,
            offset: offset,
        }
    }

    fn map_localresult(tz: Tz, result: LocalResult<FixedTimespan>) -> LocalResult<Self> {
        match result {
            LocalResult::None => LocalResult::None,
            LocalResult::Single(s) => LocalResult::Single(TzOffset::new(tz, s)),
            LocalResult::Ambiguous(a, b) => LocalResult::Ambiguous(TzOffset::new(tz, a), TzOffset::new(tz, b)),
        }
    }
}

impl Offset for TzOffset {
    fn local_minus_utc(&self) -> Duration {
        self.offset.local_minus_utc()
    }
}

impl Display for TzOffset {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        Display::fmt(&self.offset,f)
    }
}

impl Debug for TzOffset {
    fn fmt(&self, f: &mut Formatter) -> Result<(), Error> {
        Debug::fmt(&self.offset,f)
    }
}

/// Represents the span of time that a given rule is valid for.
/// Note that I have made the assumption that all ranges are
/// left-inclusive and right-exclusive - that is to say,
/// if the clocks go forward by 1 hour at 1am, the time 1am
/// does not exist in local time (the clock goes from 00:59:59
/// to 02:00:00). Likewise, if the clocks go back by one hour
/// at 2am, the clock goes from 01:59:59 to 01:00:00. This is
/// an arbitrary choice, and I could not find a source to
/// confirm whether or not this is correct.
struct Span {
    begin: Option<i64>,
    end: Option<i64>,
}

impl Span {
    fn contains(&self, x: i64) -> bool {
        match (self.begin, self.end) {
            (Some(a), Some(b)) if a <= x && x < b  => true,
            (Some(a), None   ) if a <= x           => true,
            (None   , Some(b)) if b > x            => true,
            (None   , None   )                     => true,
            _                                      => false,
        }
    }

    fn cmp(&self, x: i64) -> Ordering {
        match (self.begin, self.end) {
            (Some(a), Some(b)) if a <= x && x < b  => Ordering::Equal,
            (Some(a), Some(b)) if a <= x && b <= x => Ordering::Less,
            (Some(_), Some(_))                     => Ordering::Greater,
            (Some(a), None   ) if a <= x           => Ordering::Equal,
            (Some(_), None   )                     => Ordering::Greater,
            (None   , Some(b)) if b <= x           => Ordering::Less,
            (None   , Some(_))                     => Ordering::Equal,
            (None   , None   )                     => Ordering::Equal,
        }
    }
}

#[derive(Copy, Clone)]
pub struct FixedTimespanSet {
    pub first: FixedTimespan,
    pub rest: &'static [(i64, FixedTimespan)],
}

impl FixedTimespanSet {
    fn len(&self) -> usize {
        1 + self.rest.len()
    }

    fn utc_span(&self, index: usize) -> Span {
        debug_assert!(index < self.len());
        Span {
            begin: if index == 0 {
                None
            } else {
                Some(self.rest[index - 1].0)
            },
            end: if index == self.rest.len() {
                None
            } else {
                Some(self.rest[index].0)
            }
        }
    }

    fn local_span(&self, index: usize) -> Span {
        debug_assert!(index < self.len());
        Span {
            begin: if index == 0 {
                None
            } else {
                let span = self.rest[index - 1];
                Some(span.0 + span.1.utc_offset + span.1.dst_offset)
            },
            end: if index == self.rest.len() {
                None
            } else if index == 0 {
                Some(self.rest[index].0 + self.first.utc_offset + self.first.dst_offset)
            } else {
                Some(self.rest[index].0 + self.rest[index - 1].1.utc_offset + self.rest[index - 1].1.dst_offset)
            }
        }
    }

    fn get(&self, index: usize) -> FixedTimespan {
        debug_assert!(index < self.len());
        if index == 0 {
            self.first
        } else {
            self.rest[index - 1].1
        }
    }
}

pub trait TimeSpans {
    fn timespans(&self) -> FixedTimespanSet;
}

impl TimeZone for Tz {
    type Offset = TzOffset;

    fn from_offset(offset: &Self::Offset) -> Self { offset.tz }

    fn offset_from_local_date(&self, local: &NaiveDate) -> LocalResult<Self::Offset> {
        let earliest = self.offset_from_local_datetime(&local.and_hms(0, 0, 0));
        let latest = self.offset_from_local_datetime(&local.and_hms(23, 59, 59));
        // From the chrono docs:
        //
        // > This type should be considered ambiguous at best, due to the inherent lack of
        // > precision required for the time zone resolution. There are some guarantees on the usage
        // > of `Date<Tz>`:
        // > - If properly constructed via `TimeZone::ymd` and others without an error,
        // >   the corresponding local date should exist for at least a moment.
        // >   (It may still have a gap from the offset changes.)
        //
        // > - The `TimeZone` is free to assign *any* `Offset` to the local date,
        // >   as long as that offset did occur in given day.
        // >   For example, if `2015-03-08T01:59-08:00` is followed by `2015-03-08T03:00-07:00`,
        // >   it may produce either `2015-03-08-08:00` or `2015-03-08-07:00`
        // >   but *not* `2015-03-08+00:00` and others.
        //
        // > - Once constructed as a full `DateTime`,
        // >   `DateTime::date` and other associated methods should return those for the original `Date`.
        // >   For example, if `dt = tz.ymd(y,m,d).hms(h,n,s)` were valid, `dt.date() == tz.ymd(y,m,d)`.
        //
        // > - The date is timezone-agnostic up to one day (i.e. practically always),
        // >   so the local date and UTC date should be equal for most cases
        // >   even though the raw calculation between `NaiveDate` and `Duration` may not.
        //
        // For these reasons we return always a single offset here if we can, rather than being
        // technically correct and returning Ambiguous(_,_) on days when the clock changes. The
        // alternative is painful errors when computing unambiguous times such as
        // `TimeZone.ymd(ambiguous_date).hms(unambiguous_time)`.
        use chrono::LocalResult::*;
        match (earliest, latest) {
            (result @ Single(_), _) => result,
            (_, result @ Single(_)) => result,
            (Ambiguous(offset,_),_) => Single(offset),
            (_,Ambiguous(offset,_)) => Single(offset),
            (None, None) => None,
        }
    }

    // First search for a timespan that the local datetime falls into, then, if it exists,
    // check the two surrounding timespans (if they exist) to see if there is any ambiguity.
    fn offset_from_local_datetime(&self, local: &NaiveDateTime) -> LocalResult<Self::Offset> {
        let timestamp = local.timestamp();
        let timespans = self.timespans();
        let index = binary_search(0, timespans.len(),
            |i| timespans.local_span(i).cmp(timestamp)
        );
        TzOffset::map_localresult(*self, match index {
            Ok(0) if timespans.len() == 1 =>
                LocalResult::Single(timespans.get(0)),
            Ok(0) if timespans.local_span(1).contains(timestamp) =>
                LocalResult::Ambiguous(timespans.get(0), timespans.get(1)),
            Ok(0) =>
                LocalResult::Single(timespans.get(0)),
            Ok(i) if timespans.local_span(i - 1).contains(timestamp) =>
                LocalResult::Ambiguous(timespans.get(i - 1), timespans.get(i)),
            Ok(i) if i == timespans.len() - 1 =>
                LocalResult::Single(timespans.get(i)),
            Ok(i) if timespans.local_span(i + 1).contains(timestamp) =>
                LocalResult::Ambiguous(timespans.get(i), timespans.get(i + 1)),
            Ok(i) =>
                LocalResult::Single(timespans.get(i)),
            Err(_) => LocalResult::None,
        })
    }

    fn offset_from_utc_date(&self, utc: &NaiveDate) -> Self::Offset {
        // See comment above for why it is OK to just take any arbitrary time in the day
        self.offset_from_utc_datetime(&utc.and_hms(12, 0, 0))
    }

    // Binary search for the required timespan. Any i64 is guaranteed to fall within
    // exactly one timespan, no matter what (so the `unwrap` is safe).
    fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> Self::Offset {
        let timestamp = utc.timestamp();
        let timespans = self.timespans();
        let index = binary_search(0, timespans.len(),
            |i| timespans.utc_span(i).cmp(timestamp)
        ).unwrap();
        TzOffset::new(*self, timespans.get(index))
    }
}
