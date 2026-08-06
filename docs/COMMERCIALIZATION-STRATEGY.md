# Spectyn Mesh — Commercialization Strategy

> **Status:** Decisive recommendation. Supersedes the earlier `docs/design/COMMERCIAL-DESIGN.md`
> (which assumed an Apache-core / BSL-broker / Tailscale-clone shape). This document is
> the authoritative commercial spine from the v0.6.0 cycle onward.
>
> **Companion:** A Traditional-Chinese companion lives at
> [`COMMERCIALIZATION-STRATEGY.zh-TW.md`](COMMERCIALIZATION-STRATEGY.zh-TW.md), matching the
> bilingual convention of [`superpowers/BIG-GOAL.md`](superpowers/BIG-GOAL.md).
>
> **Constraints this strategy must respect:** mostly open source; a solo maintainer plus an
> AI-agent fleet; no venture capital; this is the maintainer's livelihood; and — non-negotiable —
> it must not violate the [Big Goal](superpowers/BIG-GOAL.md): *runs on your own devices,
> local-first, your data encrypted so only you can read it.*
>
> **⚖️ Governance calibration (2026-06-11, owner ruling (a)):** This document is **subordinate to
> the locked [`BIG-GOAL.md`](superpowers/BIG-GOAL.md)** and to the execution-sequencing in
> [`STRATEGY-DIFFERENTIATION.md`](STRATEGY-DIFFERENTIATION.md). Read it at **side-business scale**,
> not venture: the owner's call is **1 + 2 (portfolio→job AND a power-user niche side-business),
> explicitly NOT 3 (scale/venture)**. The single paid product (Spectyn Relay) is a convenience that
> *extends* P4 (carries only ciphertext it cannot read) and never paywalls a Big-Goal pillar — the
> whole cross-device mesh (P1), multimodal capture (P2), evolve/skill bank (P3), encryption (P4),
> and the mobile apps stay free forever. The **relay/commercialization door is kept open** (owner: both
> the interview-portfolio and the side-business matter), which is *why* the license recommendation
> below is **AGPL core + Apache/MIT SDK + FSL relay** day-one — preserving the business option is the
> low-cost, asymmetric-safe choice (AGPL→permissive later is safe; permissive→tighten causes revolts).
> Commercialization is the **downstream reward** of building toward the Big Goal, and must never be
> allowed to reshape what Spectyn *is*.

---

## 0. Why this document exists, and how it aligns with the Big Goal

The [Big Goal](superpowers/BIG-GOAL.md) makes four promises that are load-bearing for *every*
commercial decision below:

- **P1 — Cross-device mesh, local-first.** The product must be fully functional air-gapped, on
  your own hardware, with a local model. → *Nothing core can sit behind a paywall or a login.*
- **P2 — Multimodal understanding.** → *Anti-goal in the Big Goal itself: core multimodal must
  never be gated behind a paid tier.*
- **P3 — Evolve mesh ("grows with how you use it").** → *Gating the thing that makes Spectyn
  learn would be self-defeating; the skill bank and evolve loop stay free.*
- **P4 — Encryption-first ("your data, encrypted, only you can read").** → *Any paid service may
  only ever move encrypted bytes it cannot read; charging for encryption itself would destroy the
  trust narrative the whole product rests on.*

The recommended model is the **only** one that fits all four. It charges for one thing — a
**zero-knowledge hosted relay** — which is a strict *extension* of the P4 encryption promise (the
relay carries ciphertext it cannot decrypt), never a violation of it. The mesh stays air-gapped and
fully capable with no relay, satisfying P1's local-first anti-goal. No core capability, no mobile
app, and no multimodal or evolve feature is ever paid. The alignment is exact and deliberate.

---

## 1. The recommended model — "Nabu Casa model": free open-source core forever + one paid cloud-convenience layer

**The single primary model:** the entire mesh — engine, agents, evolve loop, encryption, and every
platform client *including the mobile apps* — is **free and open source forever**. The only paid
product is a subscription **hosted convenience service, "Spectyn Relay"**: a zero-knowledge
rendezvous/relay point for phones and laptops behind NAT, a push-notification bridge, and
end-to-end-encrypted off-site backup.

### Why this model and not the others (reasoning, all from the research evidence)

1. **It is the only model proven to sustain a solo / tiny team with zero VC.** Home Assistant's
   commercial arm, Nabu Casa, funds an entire foundation and dozens of staff from a single
   ~$6.50/mo SKU (remote relay + cloud voice + off-site backup) on a 2M+ install base, having
   **never taken external investment** [1][2]. Coolify reached ~$15.7k/mo gross, ~$12.9k/mo net,
   as a solo founder, charging $5/mo to host the control plane plus sponsorships [3][4]. Ollama
   monetizes hosted *inference convenience* on top of a free local core — a near-bootstrapped team,
   no large VC round on record [5][6].

2. **Every other model requires VC or a sales org.** n8n's fair-code + enterprise sales took a
   $180M Series C [7]; Supabase's "sell hosting" model raised >$1B because running Postgres
   reliably is genuinely hard [8][9]; Tailscale's closed-coordinator + seat-based B2B took a $160M
   Series C [10]; LM Studio keeps closed enterprise features behind VC funding. A solo maintainer
   can build none of these.

3. **The paid layer is precisely "the thing self-hosters *can* do but don't want to run 24/7."**
   Relay, rendezvous, push, off-site backup are exactly that class of chore. And because the relay
   only ferries encrypted bytes (zero-knowledge), it **does not break the Big Goal's "your data,
   only you can read it"** — it *extends* the encryption promise. The mesh stays air-gapped and
   fully functional with no relay, satisfying the local-first anti-goal.

4. **Consumer "private AI software" is already commoditized to free.** Ollama, LM Studio, Jan,
   Open WebUI are all free; prosumers will spend $1k–$15k on hardware but have near-zero tolerance
   for a *subscription just to orchestrate local models* [11]. So you must never charge for core
   capability — only for *convenience*.

5. **Individual wallets cap "convenience-on-top-of-free" at roughly $5–20/mo** (Coolify $5, Nabu
   Casa $6.50, Ollama Pro $20). Relay-class pricing belongs at the low end of that band.

### Two fallback levers (not parallel main lines)

- **Lever A — Immich-style voluntary lifetime supporter license.** Zero feature gates,
  purely supportive: a one-time license ($29 personal / $99 per cluster). It front-loads cash flow
  with zero community risk and can sell *before* the relay ships [12].
- **Lever B — Paid support / commercial license for regulated SMBs (v0.8.0+ only).** Legal,
  medical, and financial back-offices have demonstrably paid $100–500/user/year for "on-prem is the
  point" (Lemony $499/mo/node, Tabby $19–24/user/mo) [13][14]. Engage only on inbound signal — this
  one requires a sales motion.

---

## 2. The open-source / paid boundary

**Design principle:** every community revolt in the research — Plex gating mobile behind a
subscription, Emby's gated app getting eaten by Jellyfin, Open WebUI tightening its license after
the fact — came from **charging for, or later tightening, something users considered theirs by
right** [15][16]. Therefore:

| Component | Open source (free forever) | Paid | Rationale |
|---|---|---|---|
| Core engine / mesh P2P / CLI | ✅ AGPL | — | The product itself; the Big Goal forbids a paywall here |
| Four-role squad, evolve loop, skill bank | ✅ AGPL | — | "Grows with how you use it" is the Big Goal — gating it is suicide |
| Encryption (at-rest, per-device key) | ✅ AGPL | — | Charging for encryption destroys the trust narrative |
| **Mobile apps (Android / iOS client)** | ✅ Free | — | The Plex/Emby lesson: charging for a mobile client invites a fork |
| Skills marketplace / SDK | ✅ MIT/Apache | Free forever, no rev-share | HACS / VS Code / Raycast all run free marketplaces as an adoption moat [17] |
| **Spectyn Relay** (hosted rendezvous + push bridge + E2E off-site backup) | Protocol open; server code FSL | 💰 ~$6/mo | The one paid product; self-hosting your own relay stays possible forever (Tailscale's goodwill toward Headscale [18]) |
| Supporter lifetime license | — | 💰 Voluntary, no gate | Immich model |
| Enterprise SSO / RBAC / audit (future) | — | 💰 Commercial license | "Buyer-based open core": charge only for what a manager buys [19] |

**The commitment to put in writing publicly:** the complete single-user, own-devices experience —
multimodal, evolve, encryption, *and the mobile apps* — is **100% free forever**, and the protocol
for self-hosting your own relay is **always public**. The free tier is benchmarked against
Tailscale Personal (free for 6 users / unlimited devices [20]).

---

## 3. Licensing recommendation

| Component | License | Rationale |
|---|---|---|
| Core (engine / mesh / CLI / agents) | **AGPLv3** | Blocks cloud strip-mining: hyperscalers like Google have a blanket internal ban on AGPL and simply will not build a "Spectyn Mesh Cloud" [21]; for an end-user self-hoster AGPL imposes zero obligation, so the cost/benefit is unusually favorable for a local-first product. Grafana / Bitwarden / Nextcloud have proven AGPL + commercial dual-licensing. |
| Skills SDK / protocol lib / client embed layer | **Apache-2.0 or MIT** | So skill authors and integrators are not "infected" by copyleft — this is what lets a marketplace grow. |
| Relay server | **FSL-1.1-Apache-2.0** | A 2-year auto-conversion to Apache (a *Delayed Open Source Publication*) is a credible promise that still blocks a "Spectyn Relay competitor SaaS" [22][23]; avoids BSL, whose reputation went toxic after HashiCorp. |
| Contributions | **CLA (via cla-assistant), with the purpose honestly disclosed in CONTRIBUTING.md** | Preserves the dual-licensing option: the iOS App Store conflicts with AGPL, and only the sole copyright holder can self-grant a store exception. Switching DCO→CLA *later* is a trust event; CLA from day one, stated plainly, is not [24]. |
| Trademark | **Register the "Spectyn Mesh" word mark (USPTO ~$350/class) + publish a trademark policy** (forks must rename; nominative use allowed) | The highest-leverage protection a solo maintainer can afford; OpenTofu being forced to rename is the power of a trademark [25][26]. |

**The single most important rule:** *use a protective license from day one.* Every 2024–2026
community revolt (HashiCorp / Redis / Elastic / Open WebUI) was caused by "permissive first, tighten
later." No project that was AGPL from day one has suffered an equivalent revolt [27][28]. Finalize
`LICENSE` *before* the public v0.6.0 launch, and never change it after.

> **⚠️ Current repo state:** this repository ships dual `LICENSE-APACHE` + `LICENSE-MIT` today.
> Adopting AGPL core is therefore a *pre-launch relicensing* decision that must land before the
> public mirror sees a community form. It is exactly the "lock the license before the community
> exists" action this section argues for — do it now, not after.

---

## 4. Pricing draft (with comparables)

| Plan | Price | What's in it | Comparables |
|---|---|---|---|
| **Free** | $0 | Everything: unlimited own devices, every-OS node, squad, evolve, encryption, skills marketplace | Tailscale Personal (free, 6 users / unlimited devices), Ollama, Jellyfin |
| **Supporter (lifetime, voluntary)** | **$29 personal / $99 per cluster** | Zero feature difference; badge + CHANGELOG credit + early test access | Immich $24.99/person, $99.99/server; Obsidian Catalyst $25 |
| **Spectyn Relay** | **$6/mo or $60/yr** | Zero-knowledge relay (NAT traversal), push bridge, E2E off-site encrypted backup, "support development" framing | Nabu Casa $6.50/$65; Coolify $5; Obsidian Sync $4–10 band |
| **Team / Compliance (v0.8.0+, only on signal)** | ~$15/user/mo (per-seat, **never per-device**) | SSO, audit logs, priority support, commercial license | Tabby $19–24/user/mo; Msty Teams $300/user/yr; Tailscale Standard $8/user/mo |

**Pricing-unit lesson: charge per *person*, not per *device*.** Tailscale's 2026 "pricing v4"
removed the device-priced Personal Plus plan ("created friction with no real differentiation") and
ZeroTier's free-device cap cut triggered backlash [29][30].

**Honest revenue math.** The industry median free→paid conversion is ~2.6%; 3–5% is good [31];
pure donations run <1%. To reach $3k/mo (solo-livable) needs ~500 relay subscribers ≈ ~17,000
active clusters at a 3% conversion. **This is a 12–24-month road, not a 90-day one** — which is
exactly why the Supporter lifetime license (Lever A) should sell in parallel to front-load cash.

---

## 5. Operator direction — the free open-weight house model as the default brain

> Added by the maintainer on top of the research, and consistent with the Big Goal's BYOM
> (bring-your-own-model) anti-lock-in stance and P3's "12+ providers behind one trait."

**Direction:** ship a free, open-weight, house-distilled model as Spectyn's *default brain* — the
thing that makes the air-gapped, no-API-key experience genuinely good out of the box. Then **monetize
the hosted inference and the personalized distillation, never the weights themselves.**

How this fits the rest of the strategy:

- **The weights stay free and open.** Giving away a good default model is the same move Ollama/Jan
  made — it commoditizes the *runner* and makes the free tier excellent, which is the adoption moat.
  It also keeps the BYOM promise intact: the house model is a default, never a lock-in; any of the
  12+ providers still swaps in per-request.
- **What's paid is convenience and personalization, not capability:**
  - *Hosted inference* — for users who don't have, or don't want to run, a GPU box 24/7. This is the
    same "self-hosters can do it but don't want to operate it" logic as the relay, and it slots
    cleanly under (or alongside) the Spectyn Relay SKU. GPU-time billing, not token billing — the
    Ollama Cloud model [5].
  - *Personalized distillation* — Spectyn's whole thesis is "grows with how you use it" (P3). A paid
    service that distills a *personalized* small model from a user's own (encrypted, consented) skill
    bank and usage is convenience layered on the evolve loop, and the output model belongs to the
    user. The training pipeline for this is the [spectyn-training](ECOSYSTEM.md) satellite.
- **What this is *not*:** it is not selling the base weights, and it is not gating the local evolve
  loop. Distillation you run yourself, on your own hardware, stays free. You pay only if you want
  *us* to run the GPU and the pipeline for you.

This keeps the model consistent: **free open core (now including a free default brain), one paid
zero-knowledge convenience layer (relay + optional hosted/personalized inference), and a voluntary
supporter license.** Nothing the user *is entitled to* ever moves behind a paywall.

---

## 6. Go-to-market — the 90-day sequence

**Phase 0 — before v0.6.0 ships (now → launch, ~4 days): legal foundation before features.** On
the public repo, finalize `LICENSE` (AGPL core), `TRADEMARK.md`, `SUSTAINABILITY.md` (the public
boundary commitment + honest CLA disclosure), and the CLA bot. **These must be in place before a
community forms** — adding them after the fact is itself a revolt trigger.

**Phase 1 — launch (first ~4 weeks):** the killer asset is the already-proven four-platform
federation demo (Windows + Linux + Android + Mac mesh). Record a 2-minute video: *"snap a photo of
your lunch, the coach on your phone replies, and the inference runs on your home desktop — encrypted
end to end."* Post to Show HN, r/selfhosted, r/LocalLLaMA, r/homelab, and Lobsters (staggered 2–3
days apart). This audience is exactly the homelab early adopter who buys $1k–$10k of hardware and
demands the software be free — a perfect fit for the free-core strategy. In parallel, open GitHub
Sponsors + a merchant-of-record (Polar: 5% + 50¢ MoR, can sell license keys [32]) to sell the
Supporter license, and stand up a Relay waitlist landing page.

**Phase 2 — v0.7.0 (~Q3):** extend encryption to `agents.toml` / conversations / `memory.db`
(completing the P4 narrative), plus a Relay MVP (rendezvous server on a $5 VPS, serving the waitlist
first). The relay solves precisely the NAT-behind pain that blocks the Big Goal's "command it from
your phone or browser" promise — product and business sit on the same road.

**Scale decision metric (read at day 90): Relay waitlist sign-ups.** ≥200 → go all-in on relay GA;
50–200 → slow beta; <50 → pivot the main effort to Levers A+B and demote the relay to a self-use
feature. Supporting metrics: GitHub stars ≥2,000, cumulative Supporter revenue ≥$500.

### 90-day action checklist (ordered; 🤖 = automatable by the agent fleet)

**Week 1 (pre-ship):**
1. 🤖 Add AGPLv3 `LICENSE` to the core repo; relicense the SDK sub-crate to Apache-2.0.
2. 🤖 Write `SUSTAINABILITY.md` (boundary commitment + CLA disclosure) and a `TRADEMARK.md` draft
   (agent drafts; maintainer signs off).
3. 🤖 Stand up the cla-assistant bot.
4. 🤖 Scrub the public repo per the public-leak protocol.
5. 🤖 Rewrite the README: tagline *"Your AI mesh. Your data. Your devices."* + a one-line install +
   a demo GIF.
6. Ship v0.6.0.

**Weeks 2–4 (launch):**
7. Record the four-platform federation demo video (maintainer records; agents draft the script /
   edit list).
8. 🤖 Draft the Show HN copy; the maintainer posts it personally and works the comments for 48 hours
   (HN replies cannot be outsourced — the voice must be the maintainer's).
9. 🤖 Draft r/selfhosted, r/LocalLLaMA, r/homelab posts, staggered.
10. Open GitHub Sponsors + Polar; list the Supporter license at $29/$99.
11. 🤖 Build the Relay waitlist landing page (static page + form).
12. 🤖 Stand up a quick-start docs site (mdBook on GitHub Pages).

**Weeks 5–12 (compounding):**
13. 🤖 Weekly devlog (agent first draft, maintainer final) to blog + Reddit.
14. 🤖 Issue triage with <24h response (agent fleet rotation, fits the no-idle loop).
15. 🤖 Seed 10 demo skills into the skill bank (marketplace cold-start).
16. File the USPTO trademark — a one-time consult with an OSS-literate attorney (~$1–2k, **the only
    line item that needs cash**); confirm the CLA wording + App Store exception at the same time.
17. 🤖 Relay MVP: rendezvous server (reusing mesh code) deployed to a $5 VPS, under FSL.
18. 🤖 Comparison page (vs Ollama / Jan / AnythingLLM: "they're single-machine runners; we're a
    cross-device mesh").
19. 🤖 Automated release pipeline (5-platform binaries — lowers the bus factor).
20. Day 90: run the go/no-go decision against the metrics in this section.

---

## 7. Risk table

| # | Risk | Mitigation |
|---|---|---|
| 1 | **Strip-mining:** someone ships a "Spectyn Mesh Cloud" and captures the monetization layer | AGPL core (hyperscaler policy ban) + registered trademark (a competitor can't use your name) + FSL relay server (no competitive use for 2 years). The three layers stack into the combined deterrent the research concludes on [21][22]. |
| 2 | **Solo bus-factor:** if the maintainer is out, the project and the revenue go to zero — so users hesitate to pay | Short term: automate release/triage + publish the ops runbook. Mid term: promote 1–2 co-maintainers from the community. Hard-coded promise: the relay code carries an FSL DOSP (if the company dies, the code auto-converts to Apache) — more credible than a verbal promise [23]. *Honest note: this is the risk this strategy can least fully eliminate.* |
| 3 | **OSS community revolt** (read as crippled-core, or fear of a future rug-pull) | The only reliable defense is "never tighten after the fact": day-one AGPL, a public boundary commitment, honest CLA disclosure, a zero-gate Supporter license, mobile apps free forever. Every revolt on record was a *post-hoc* license change; there is no precedent of a day-one protective license being revolted against [28]. |
| 4 | **Conversion too low to sustain a livelihood** (the most existential risk) | Face it honestly: 2–5% is the real band, donations <1%. Use Lever A (lifetime license) to front-load cash; keep other income until MRR is stable; if the 90-day metric misses, pivot immediately to Lever B (SMB support, $100–500/user/yr) rather than burning more on relay. Stated willingness-to-pay for privacy vastly exceeds *revealed* willingness (the privacy paradox) — trust only the waitlist and the card swipes. |
| 5 | **AGPL friction:** enterprise legal bans deter potential contributors/embedders; the iOS App Store won't accept AGPL | Use Apache/MIT for the SDK/protocol layer (integrations aren't infected); the CLA preserves dual-licensing — the sole copyright holder can self-grant an App Store exception and commercial licenses, and when real enterprise demand appears this *becomes* the Lever-B paid entry point. The App Store exception wording needs a lawyer's eyes (jurisprudence is thin) [33]. |

---

## 8. Honest caveats

This is a strategy memo, not legal or financial advice. The evidence has real limits:

- **The anchor case is inferred, not disclosed.** Nabu Casa's subscriber count and revenue are not
  public; the ~5% conversion / ~100k-subscriber figure is an inference from "50+ FTE funded mainly
  by subscriptions" + "2M homes." Coolify's numbers are founder-self-reported on X.
- **Conversion benchmarks are borrowed.** The 2.6% median / 3–5% "good" band is general B2B-SaaS
  freemium data; **no public conversion data exists for self-hosted open-source products
  specifically.** The revenue math here could be off by a factor of two in either direction.
- **Private-company revenue figures are estimates.** Tailscale / n8n / Supabase ARR come from
  estimate aggregators (getlatka, Sacra) and are not audited.
- **Pricing in this market is volatile.** DGX Spark rose ~$700 within weeks of launch; Tailscale's
  April-2026 "pricing v4" removed plans that mid-2025 sources still cite; Bitwarden's iconic $10/yr
  Premium ended ~Jan 2026. Treat every quoted price as a mid-2026 snapshot.
- **The regulated-vertical opportunity is a gap claim, not an existing pool.** Most paid "private
  AI" for doctors/lawyers today is compliance-wrapped *cloud* SaaS, not local inference; the
  truly-local paid market in those verticals is currently small-ticket utilities. Lever B is a bet
  on a gap, not a tap on existing revenue.
- **Privacy willingness-to-pay is soft evidence.** The strongest privacy-premium survey numbers come
  from sources with an interest in the conclusion; the hard revealed-preference evidence is hardware
  sellouts and on-prem contracts, not survey percentages.
- **Legal mechanics need a real review.** Trademark, CLA wording, the App Store/AGPL exception
  (jurisprudence is thin), and FSL clauses all warrant one OSS-literate-attorney consult before any
  public commitment (it's action-item #16).
- **The research is US-centric.** WebSearch is US-only; EU digital-sovereignty dynamics (which favor
  AGPL) and Taiwan/APAC paying behavior are under-represented — though the early community
  (HN / r/selfhosted) is global English-speaking anyway.

---

## One-sentence summary

**Nail "runs on all your devices, encrypted so only you can read it" as free forever with a day-one
AGPL + trademark, then sell exactly one thing — a ~$6/mo zero-knowledge Spectyn Relay (NAT traversal
+ push + encrypted off-site backup, optionally bundled with hosted/personalized inference on a free
house-distilled model) — the path Nabu Casa has already proven can sustain a whole team with zero
funding, front-loaded by a $29/$99 lifetime supporter license, with the relay waitlist count
deciding at day 90 whether to double down or pivot.**

---

## References

Source list from the underlying commercialization research (≈130 cited URLs; the bracketed numbers
above map to this list). Figures attributed to these sources carry the caveats in §8.

1. Nabu Casa pricing — https://www.nabucasa.com/pricing/
2. State of the Open Home 2025 — https://www.home-assistant.io/blog/2025/04/16/state-of-the-open-home-recap/
3. Coolify founder revenue update — https://x.com/heyandras/status/1901894087604916396
4. Coolify pricing / philosophy — https://coolify.io/pricing · https://coolify.io/philosophy
5. Ollama pricing (GPU-time hosted inference) — https://ollama.com/pricing
6. Ollama repo / pre-seed funding — https://github.com/ollama/ollama · https://www.trysignalbase.com/news/funding/ollama-lands-125k-in-pre-seed-funding-to-accelerate-large-language-model-integration
7. n8n Series C — https://blog.n8n.io/series-c/
8. Supabase $5B valuation — https://techcrunch.com/2025/10/03/supabase-nabs-5b-valuation-four-months-after-hitting-2b/
9. Supabase pricing — https://supabase.com/pricing
10. Tailscale free plan / funding — https://tailscale.com/blog/free-plan · https://getlatka.com/companies/tailscale.com
11. NVIDIA DGX Spark ($3,999 dev PC) — https://www.engadget.com/ai/nvidia-starts-selling-its-3999-dgx-spark-ai-developer-pc-120034479.html
12. Immich license / lifetime model — https://github.com/immich-app/immich/discussions/11186
13. Lemony AI (on-prem box, $499/mo/node) — https://techcrunch.com/2025/06/11/uptime-industries-wants-to-boost-localized-ai-usage-with-an-ai-in-a-box-called-lemony-ai/
14. Tabby pricing ($19–24/user/mo) — https://www.tabbyml.com/pricing
15. Plex 2025 updates (mobile gating) — https://www.plex.tv/blog/important-2025-plex-updates/
16. Open WebUI license change + backlash — https://docs.openwebui.com/license/ · https://biggo.com/news/202511041923_open-webui-license-change-backlash
17. Raycast pricing (free marketplace) — https://www.raycast.com/pricing
18. Tailscale open source / Headscale tolerance — https://tailscale.com/opensource · https://github.com/juanfont/headscale
19. Open core vs crippled core (buyer-based) — https://peterzaitsev.com/open-source-business-models-open-core-vs-crippled-core/
20. Tailscale pricing v4 (per-user, free tier) — https://tailscale.com/blog/pricing-v4 · https://tailscale.com/pricing
21. Google AGPL policy (hyperscaler ban) — https://opensource.google/documentation/reference/using/agpl-policy
22. FSL (Functional Source License) — https://fsl.software/ · https://blog.sentry.io/sentry-is-now-fair-source/
23. FSL vs AGPL for open-source businesses — https://lucumr.pocoo.org/2024/9/23/fsl-agpl-open-source-businesses/
24. DCO vs CLA management — https://tenthirtyam.org/dispatches/2026/04/08/dco-vs-cla-managing-contribution-agreements-in-open-source/ · https://drewdevault.com/2021/04/12/DCO.html
25. USPTO trademark cost — https://www.uspto.gov/trademarks/basics/how-much-does-it-cost
26. OpenTofu fork / trademark power — https://opentofu.org/blog/opentofu-announces-fork-of-terraform/ · https://www.linuxfoundation.org/blog/blog/open-source-communities-and-trademarks-a-reprise
27. Redis AGPL relicense — https://www.infoq.com/news/2025/05/redis-agpl-license/ · https://www.elastic.co/blog/elasticsearch-is-open-source-again
28. Community erosion after license change (Percona) — https://www.percona.com/blog/community-erosion-post-license-change-quantifying-the-power-of-open-source/
29. ZeroTier free-device cap backlash — https://news.ycombinator.com/item?id=41127757 · https://www.zerotier.com/news/introducing-our-new-usage-based-pricing-model-zerotier-essential/
30. AGPL is a non-starter for most companies — https://www.opencoreventures.com/blog/agpl-license-is-a-non-starter-for-most-companies
31. SaaS freemium conversion rates — https://firstpagesage.com/seo-blog/saas-freemium-conversion-rates/ · https://chartmogul.com/reports/saas-conversion-report/
32. Polar (merchant of record) — https://polar.sh/resources/why
33. AGPL & App Store distribution conflict — https://www.opencoreventures.com/blog/agpl-license-is-a-non-starter-for-most-companies

**Additional comparables and evidence consulted** (the rest of the ~130-source corpus): Ollama vs
LM Studio vs Jan runners; AnythingLLM, Dify, Msty, MacWhisper/superwhisper local-AI utilities;
Cohere North, Mistral/CMA-CGM, and enterprise on-prem repatriation surveys; AI medical-scribe and
legal-AI pricing; the RedMonk / OSI / Lago / Liquibase / Sentry licensing analyses; OpenTofu /
Elastic / Redis relicense post-mortems; Plex / Emby / Immich / Portainer / Proxmox / Bitwarden /
Obsidian tier comparables; ZeroTier and Tailscale pricing changes; GitHub Sponsors playbooks
(Caleb Porzio), Discord/Polar/Dodo monetization rails; and product-led-growth / freemium-conversion
benchmarks. The full URL list (with per-record findings and caveats) lives in the research artifact
that backs this document.

---

## Internal notes (strip before public mirror) / 內部備註（公開鏡像前移除）

<!-- INTERNAL: everything below this line must be removed or sanitized before this file reaches the public mirror (markl-a/spectyn-mesh). -->

- **Sequencing vs other tracks:** Phase-0 legal foundation is a 4-day window before the v0.6.0
  public push. It is independent of feature tracks and can be fanned out across the agent fleet in
  parallel; only the trademark filing (#16) blocks on the maintainer + an external attorney.
- **Concrete public-repo targets:** the relicensing in §3 and the README rewrite in §6 land in the
  public mirror; do the public-leak scrub (E-epic/F-feature IDs, internal node names, fleet IPs,
  internal absolute paths) before pushing. The dual `LICENSE-APACHE` + `LICENSE-MIT` files in this
  repo root are what need to be replaced with the AGPL core decision.
- **Satellite tie-in:** the personalized-distillation SKU in §5 is implemented by the
  `spectyn-training` satellite (see `docs/ECOSYSTEM.md`); the relay rendezvous server reuses the
  existing mesh RPC code, and the OAuth/login broker work is the `phantommesh.io` Cloudflare Worker
  already described in `docs/commercial/CONTRIBUTOR-FUNNEL.md`.
- **Decision artifact:** this document is the polished form of the decisive internal strategy memo;
  the underlying research artifact (≈130 sources with per-record findings/caveats) is retained in
  the operator's plans directory, not in the repo.
```
