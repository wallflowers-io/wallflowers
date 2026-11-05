//  ChromodynamicsTests.swift
//  The three charges proven on plain language — deterministic, no model.
//
//  What must hold: retrospective text reads RED, prospective text reads BLUE,
//  rich text carries more GOLD than repetition; balanced charge sets compose
//  toward WHITE and single-charge sets do not; and the whole instrument is
//  deterministic run to run.

import Testing
@testable import PacificStore

struct ChromodynamicsTests {

    @Test func retrospectiveTextReadsRed() {
        let c = Chromodynamics.charge(
            "we were there last summer, remember how it began and how it ended")
        #expect(c.red > c.blue)
        #expect(c.red > 0.3)
    }

    @Test func prospectiveTextReadsBlue() {
        let c = Chromodynamics.charge(
            "let's plan the launch for next month, we will meet soon and book the venue")
        #expect(c.blue > c.red)
        #expect(c.blue > 0.3)
    }

    @Test func richTextCarriesMoreGoldThanRepetition() {
        let rich = Chromodynamics.charge(
            "the harbour market gathers ceramicists, luthiers and foragers under one tide chart")
        let flat = Chromodynamics.charge(
            "yes yes yes yes yes yes yes yes yes yes yes yes")
        #expect(rich.gold > flat.gold)
    }

    @Test func balancedChargesComposeTowardWhite() {
        let balanced = Chromodynamics.compose([
            Charge(red: 0.8, blue: 0.1, gold: 0.1),
            Charge(red: 0.1, blue: 0.8, gold: 0.1),
            Charge(red: 0.1, blue: 0.1, gold: 0.8),
        ])
        let pure = Charge(red: 0.9, blue: 0, gold: 0)
        #expect(balanced.whiteness > 0.95)
        #expect(pure.whiteness < 0.1)
    }

    @Test func neutralIsVacuouslyWhiteAndEmptyComposesNeutral() {
        #expect(Charge.neutral.whiteness == 1)
        #expect(Chromodynamics.compose([]) == .neutral)
    }

    @Test func deterministicRunToRun() {
        let text = "we came from the old studio and now we will build the new one"
        #expect(Chromodynamics.charge(text) == Chromodynamics.charge(text))
    }

    @Test func emptyTextIsNeutral() {
        #expect(Chromodynamics.charge("   ") == .neutral)
    }
}

/// lc-kr-v1: the Korean instrument,
/// proven on plain Korean — affix-matched particles/endings, deterministic, no
/// model. The whole lexicon is agent-drafted pending native-speaker review; these
/// fixtures pin the MECHANICS (which affix classes fire and which charge wins),
/// not the lexicon's linguistic completeness.
struct ChromodynamicsKoreanTests {

    // "We came from Seoul yesterday — it was hard because of last summer."
    // 어제 (exact) + 서울에서 (particle suffix) + 왔어 (past verb) + 지난 (exact)
    // + 때문에 — the retrospective charge, through eojeol the en instrument
    // cannot see at all.
    @Test func retrospectiveKoreanReadsRed() {
        let c = Chromodynamics.charge(
            "우리는 어제 서울에서 왔어, 지난 여름 때문에 힘들었지", language: .korean)
        #expect(c.red > c.blue)
        #expect(c.red >= c.gold)
        #expect(c.red > 0.5)
    }

    // "Tomorrow I plan to go to the festival with friends; let's plan for next
    // week's show." 내일/갈/다음/위해 (exact) + 예정이야/계획을 (content word
    // wearing a particle — the prefix affix class).
    @Test func prospectiveKoreanReadsBlue() {
        let c = Chromodynamics.charge(
            "내일 친구들과 함께 축제에 갈 예정이야, 다음 주 공연을 위해 계획을 세우자", language: .korean)
        #expect(c.blue > c.red)
        #expect(c.blue >= c.gold)
        #expect(c.blue > 0.5)
    }

    // "This report covers festival-related information and the event's contents."
    // 관련/정보/내용/대해 — aboutness markers RAISE gold in lc-kr-v1; with no
    // directional marker in sight, gold is strictly dominant.
    @Test func aboutnessKoreanReadsGold() {
        let c = Chromodynamics.charge(
            "이 보고서는 축제 관련 정보 와 행사 내용 에 대해 다룬다", language: .korean)
        #expect(c.gold > c.red)
        #expect(c.gold > c.blue)
        #expect(c.red < 0.2)
        #expect(c.blue < 0.2)
    }

    // The en instrument on the SAME Korean sentences: the §12 F-2 finding —
    // red/blue lexicons fire on none, charges collapse to entropy-gold. Pinned
    // so the failure lc-kr exists to fix stays visible.
    @Test func englishInstrumentIsBlindToKorean() {
        let red = Chromodynamics.charge("우리는 어제 서울에서 왔어, 지난 여름 때문에 힘들었지")
        let blue = Chromodynamics.charge("내일 친구들과 함께 축제에 갈 예정이야")
        #expect(red.red == 0)
        #expect(blue.blue == 0)
    }

    // lc-kr-v1 is a SUPERSET of lc-v1, never a divergence: on pure English text
    // the two instruments are value-identical (Hangul affixes cannot fire on
    // Latin tokens; marker-boosted gold reduces to the entropy blend). Mixed
    // Seoul streams can therefore run ONE instrument.
    @Test func koreanInstrumentIsIdenticalOnEnglishText() {
        for text in [
            "we were there last summer, remember how it began and how it ended",
            "let's plan the launch for next month, we will meet soon and book the venue",
            "the harbour market gathers ceramicists, luthiers and foragers under one tide chart",
        ] {
            #expect(Chromodynamics.charge(text, language: .korean)
                    == Chromodynamics.charge(text, language: .english))
        }
    }

    @Test func deterministicRunToRunInKorean() {
        let text = "어제 부산에서 왔고 내일 서울로 갈 예정이야"
        #expect(Chromodynamics.charge(text, language: .korean)
                == Chromodynamics.charge(text, language: .korean))
    }

    // The instrument names its ruler — provenance exactly like lc-v1's.
    @Test func lexiconNamesFollowTheLanguage() {
        #expect(Chromodynamics.lexicon(for: .english) == "lc-v1")
        #expect(Chromodynamics.lexicon(for: .korean) == "lc-kr-v1")
        #expect(Chromodynamics.lexicon == "lc-v1")
    }
}
