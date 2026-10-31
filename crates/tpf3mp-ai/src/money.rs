//! The AI's money (docs/AI_PLAYERS.md, "Money"), as a player keeps it: what
//! its lines earn and its loans cost a month, what it keeps in hand for the
//! payments to come, whether a loan pays its way, and which loan to take.
//!
//! A room's loans are paid back each of the game's months, interest and a
//! part of what is owed together (tpf3mp/companies.lua, "loans"). An AI
//! that borrowed whenever it ran short, whatever its lines earned, owed
//! 86 million in the long run of 2026-10-30, paying 8 million in interest a
//! month on lines that earned under one (investigation/AI_PLAYERS_2026-10-29.md,
//! "Money").

use tpf3mp_bridge::ai::AiCompany;
use tpf3mp_proto::action::LoanTerms;

/// How many months of loan payments it keeps in hand before it spends: the
/// payments fall due whatever its lines earn.
pub const CUSHION_MONTHS: i64 = 4;
/// Cash beyond its cushion and a loan's whole remainder by this much, it
/// pays that loan back early, the dearest first.
pub const REPAY_SPARE: i64 = 3_000_000;

/// Its money a month, from what the game says of its company.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Books {
    /// What its lines earn a month, their running costs taken off: their
    /// profit over the last twelve months, a twelfth of it.
    pub earning: i64,
    /// What its loans take a month: each one's payment.
    pub owing: i64,
}

impl Books {
    pub fn of(company: &AiCompany) -> Self {
        Self {
            earning: company.lines.iter().map(|l| l.profit).sum::<i64>() / 12,
            owing: company.owed.iter().map(|l| l.payment).sum(),
        }
    }

    /// What it must keep in hand: its next months' payments, and what its
    /// lines lose over them where they lose money together. Its loans'
    /// payments alone left the long run's AIs spending what their losing
    /// lines then took, below nothing (2026-10-31).
    pub fn cushion(&self) -> i64 {
        (self.owing + (-self.earning).max(0)) * CUSHION_MONTHS
    }

    /// Whether what its lines earn covers what its loans take.
    pub fn covered(&self) -> bool {
        self.earning >= self.owing
    }
}

/// The payment each month that pays back `amount` in `months` at `rate` a
/// month: an annuity, as the room charges a loan (companies.lua, `annuity`).
#[allow(clippy::cast_possible_truncation, clippy::cast_precision_loss)]
pub fn annuity(amount: i64, rate: f64, months: i64) -> i64 {
    let months = months.max(1);
    if rate <= 0.0 {
        return (amount + months - 1) / months;
    }
    let a = amount as f64;
    let n = i32::try_from(months).unwrap_or(i32::MAX);
    (a * rate / (1.0 - (1.0 + rate).powi(-n))).ceil() as i64
}

/// What an offered loan would take each month, the game's month `month`
/// long in its milliseconds; none where that is unknown.
#[allow(clippy::cast_precision_loss)]
pub fn payment(offer: &LoanTerms, month: u32) -> Option<i64> {
    if month == 0 || offer.amount <= 0 || offer.duration <= 0 {
        return None;
    }
    let months = (offer.duration as f64 / f64::from(month)).round().max(1.0);
    // A year's interest in millionths, a month's as a fraction.
    let rate = offer.percentage as f64 / 1e6 / 12.0;
    #[allow(clippy::cast_possible_truncation)]
    Some(annuity(offer.amount, rate, months as i64))
}

/// The loan to take, if any. To start, one whose payments its first lines
/// can carry: the largest (`big`) of those that cost little a month for
/// what they lend (within [`DEAR_START`] of the cheapest a million), or the
/// one that takes least a month; a loan paid back within the year takes
/// so much a month that no new line carries it (the long run's 5 million
/// over 12 months took 425,750 a month, 2026-10-31). Once it runs lines,
/// only one whose payment what its lines earn over what its loans take
/// already covers `cover` times over, of those the largest (`big`) or the
/// smallest. A company whose lines do not pay borrows nothing more:
/// borrowing to pay its loans is how the AI ran into debt.
pub fn offer_to_take(
    company: &AiCompany,
    books: &Books,
    big: bool,
    cover: f64,
) -> Option<LoanTerms> {
    let starting = company.lines.is_empty() && company.owed.is_empty();
    if starting && let Some(offer) = start_loan(company, big) {
        return Some(offer);
    }
    let spare = books.earning - books.owing;
    let fits = |o: &&LoanTerms| {
        if starting {
            return true;
        }
        #[allow(clippy::cast_precision_loss)]
        payment(o, company.month).is_some_and(|p| spare as f64 >= cover * p as f64)
    };
    let fitting = company.offers.iter().filter(fits);
    if big {
        fitting.max_by_key(|o| o.amount)
    } else {
        fitting.min_by_key(|o| o.amount)
    }
    .cloned()
}

/// How many times the cheapest a month for what it lends a loan to start
/// may cost, for the largest to be taken.
pub const DEAR_START: f64 = 1.5;

/// The loan to start with, where the game's month says what each costs a
/// month; none where it does not.
#[allow(clippy::cast_precision_loss)]
fn start_loan(company: &AiCompany, big: bool) -> Option<LoanTerms> {
    let priced: Vec<(&LoanTerms, i64)> = company
        .offers
        .iter()
        .filter_map(|o| Some((o, payment(o, company.month)?)))
        .collect();
    if big {
        let per = |(o, p): &(&LoanTerms, i64)| *p as f64 / o.amount as f64;
        let cheapest = priced.iter().map(per).min_by(f64::total_cmp)?;
        priced
            .iter()
            .filter(|x| per(x) <= cheapest * DEAR_START)
            .max_by_key(|(o, _)| o.amount)
    } else {
        priced.iter().min_by_key(|(_, p)| *p)
    }
    .map(|(o, _)| (*o).clone())
}

/// The loan to pay back early, if any: the dearest whose whole remainder
/// its cash covers beyond its cushion by [`REPAY_SPARE`], while its lines
/// cover its loans.
pub fn loan_to_repay(company: &AiCompany, books: &Books) -> Option<LoanTerms> {
    if !books.covered() {
        return None;
    }
    company
        .owed
        .iter()
        .filter(|l| company.money - books.cushion() - l.remaining >= REPAY_SPARE)
        .max_by_key(|l| l.terms.percentage)
        .map(|l| l.terms.clone())
}

#[cfg(test)]
mod tests {
    use tpf3mp_bridge::ai::{AiLine, AiLoan};
    use tpf3mp_proto::{BoundedVec, Text, action::LineId};

    use super::*;

    fn terms(amount: i64, months: i64, year_percent: i64) -> LoanTerms {
        LoanTerms {
            kind: Text::new("Large").unwrap(),
            amount,
            duration: months * 121_750,
            percentage: year_percent * 10_000,
            birth_day: None,
            cooldown_until: None,
            last_pay_day: None,
            times_paid: None,
            id: Some(1),
        }
    }

    fn company(money: i64, profits: &[i64], owed: &[(i64, i64)]) -> AiCompany {
        AiCompany {
            company: Some(1),
            money,
            year: 1926,
            stations: BoundedVec::default(),
            depots: BoundedVec::default(),
            lines: BoundedVec::new(
                profits
                    .iter()
                    .enumerate()
                    .map(|(i, p)| AiLine {
                        id: LineId(u32::try_from(i).unwrap()),
                        name: Text::new("Line").unwrap(),
                        stops: BoundedVec::default(),
                        vehicles: BoundedVec::default(),
                        load: 50,
                        waiting: 0,
                        profit: *p,
                        income: None,
                        problems: 0,
                    })
                    .collect(),
            )
            .unwrap(),
            vehicles: BoundedVec::default(),
            offers: BoundedVec::new(vec![terms(500_000, 6, 5), terms(27_000_000, 144, 10)])
                .unwrap(),
            loans: u8::try_from(owed.len()).unwrap(),
            owed: BoundedVec::new(
                owed.iter()
                    .map(|(remaining, payment)| AiLoan {
                        terms: terms(27_000_000, 144, 10),
                        remaining: *remaining,
                        payment: *payment,
                        months_left: 100,
                    })
                    .collect(),
            )
            .unwrap(),
            month: 121_750,
        }
    }

    #[test]
    fn a_loans_payment_is_the_rooms_annuity() {
        // The room's: 27 million over 144 months at 10 % a year, 322,672
        // a month as the long run's first loan (rate 0.00833 a month).
        assert_eq!(payment(&terms(27_000_000, 144, 10), 121_750), Some(322_672));
        assert_eq!(annuity(1_200, 0.0, 12), 100);
        assert_eq!(payment(&terms(500_000, 6, 5), 0), None, "a month unknown");
    }

    #[test]
    fn it_borrows_to_start_and_then_only_what_its_lines_carry() {
        // Starting: no lines, no loans: the largest for Hard.
        let start = company(0, &[], &[]);
        let books = Books::of(&start);
        assert_eq!(
            offer_to_take(&start, &books, true, 1.25).map(|o| o.amount),
            Some(27_000_000)
        );
        // Its lines lose money: nothing more, however short it is.
        let losing = company(100_000, &[-1_200_000, -600_000], &[(20_000_000, 300_000)]);
        let books = Books::of(&losing);
        assert!(!books.covered());
        assert_eq!(offer_to_take(&losing, &books, true, 1.25), None);
        // Its lines earn 1.2 million a month over its loans' 300,000: the
        // small loan's 84,000 a month is carried, the large one's 322,672
        // too at 1.25 times over (900,000 spare).
        let earning = company(100_000, &[14_400_000], &[(20_000_000, 300_000)]);
        let books = Books::of(&earning);
        assert_eq!(books.earning, 1_200_000);
        assert_eq!(
            offer_to_take(&earning, &books, true, 1.25).map(|o| o.amount),
            Some(27_000_000)
        );
        // At three times over, only the small one.
        assert_eq!(
            offer_to_take(&earning, &books, true, 3.0).map(|o| o.amount),
            Some(500_000)
        );
    }

    #[test]
    fn it_starts_on_a_loan_its_first_lines_can_carry() {
        // The long run's offers: 5 million paid back in a year, 6 over
        // three years, 27 over twelve, 23 over fourteen.
        let mut start = company(0, &[], &[]);
        start.offers = BoundedVec::new(vec![
            terms(5_000_000, 12, 4),
            terms(6_000_000, 36, 3),
            terms(27_000_000, 144, 10),
            terms(23_000_000, 168, 10),
        ])
        .unwrap();
        let books = Books::of(&start);
        // Hard: the largest of the long, cheap ones.
        assert_eq!(
            offer_to_take(&start, &books, true, 1.25).map(|o| o.amount),
            Some(27_000_000)
        );
        // Normal: the one that takes least a month, never the year's
        // 425,750 a month.
        let small = offer_to_take(&start, &books, false, 2.0).unwrap();
        assert_eq!(small.amount, 6_000_000);
        assert!(payment(&small, start.month).unwrap() < 200_000);
    }

    #[test]
    fn it_keeps_its_losses_in_hand_as_well_as_its_payments() {
        // Its lines lose 1.2 million a year together, 100,000 a month: four
        // months of that beside four of its loan's 300,000.
        let losing = company(5_000_000, &[-1_200_000], &[(20_000_000, 300_000)]);
        assert_eq!(Books::of(&losing).cushion(), 1_600_000);
        // Earning, only the payments.
        let earning = company(5_000_000, &[1_200_000], &[(20_000_000, 300_000)]);
        assert_eq!(Books::of(&earning).cushion(), 1_200_000);
    }

    #[test]
    fn it_keeps_its_next_payments_in_hand_and_pays_back_when_flush() {
        let flush = company(30_000_000, &[24_000_000], &[(10_000_000, 400_000)]);
        let books = Books::of(&flush);
        assert_eq!(books.cushion(), 1_600_000);
        // 30 million less 1.6 cushion less the 10 owed leaves over 3.
        assert!(loan_to_repay(&flush, &books).is_some());
        let tight = company(12_000_000, &[24_000_000], &[(10_000_000, 400_000)]);
        assert_eq!(loan_to_repay(&tight, &Books::of(&tight)), None);
        // Never while its lines do not cover its loans.
        let losing = company(30_000_000, &[-1_000_000], &[(10_000_000, 400_000)]);
        assert_eq!(loan_to_repay(&losing, &Books::of(&losing)), None);
    }
}
