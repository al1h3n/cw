# Go-to-market — first client, first 10, pricing

> Mindset: three-month horizon, hypotheses you can test fast, first clients won by hand and for free.
> Market decisions D1–D3 are in [`../AGENTS.md`](../AGENTS.md). Re-check the competitor data below
> every 6 months — older data is history, not truth.

## 1. The honest picture

- **"Open-source AnyDesk" already exists: [RustDesk](https://rustdesk.com/pricing/)** (1.4.6, March
  2026; its Pro server costs roughly $12–20/month for small setups). **"Free classroom control" also
  already exists: [Veyon](https://veyon.io/en/)** (GPL, LAN-focused, Windows/Linux). Neither is our pitch.
  Our pitch is what neither does well: **internet + offline enforcement + exam tools + the local
  language + AI.**
- **Public K-12 is the hardest place to get money** (tenders, tiny budgets, slow). It is also the
  place with the most users and the loudest word of mouth. Plan for it: teachers adopt it for free,
  the principal pays a small amount, and district deals come later. **In the first 3 months, success =
  one real school using it every day, not revenue.**
- **Open-core reality:** anyone can compile the AGPL build without the 10-device limit. People who do
  that were never customers. Schools pay for signed official builds, the hosted Hub, updates, support
  and the AI — so make those clearly better.

## 2. The burger: features → teacher language

| What we built | What the teacher hears |
|---------------|------------------------|
| Thumbnail grid, change-only JPEG | "See every screen in the room without walking the rows." |
| Process blocker + browser policies | "Games close themselves. YouTube won't open." |
| Lock desktop | "Exam? Every PC locked with one button — no Alt+Tab tricks." |
| File baseline + collect + wipe | "Collect everyone's work in one click. Wipe the lab for the next class." |
| Preloaded synced audio | "Listening test plays once, for everyone at the same second." |
| Offline Policy enforcement | "Works even when the school internet is down." |
| Visible indicator (D3) | "Honest control, not spyware. Students know the rules." |

**The emotion to sell:** the moment the teacher turns around and half the class is in Minecraft or
CS2; the anxiety of an exam where you can't see 30 screens at once. **The founder's opinion (a product
without one is doomed):** *"Computers in class are for learning. The teacher should be in control —
openly, not by spying."* Put it on the landing page as-is.

Draft headlines (Russian first, since that's the market; no product name yet):
- «Весь класс на одном экране. Игры закрываются сами.» — *The whole class on one screen. Games close themselves.*
- «Контрольная? Все компьютеры блокируются одной кнопкой.» — *Exam? Every PC locked with one button.*
- «Соберите работы всех учеников и очистите класс — в один клик.» — *Collect everyone's work and wipe the lab in one click.*

## 3. Competitors (fresh data, Sept 2026; prices are proxies, verify in your country)

| Competitor | Stage / signal | Price signal | Where we beat them |
|------------|----------------|--------------|--------------------|
| [Veyon](https://veyon.io/en/) | Mature, still maintained, free | $0 (GPL) | Internet/ID, offline Policies, exam tools, modern UI, AI |
| [LanSchool Air](https://lanschool.com/solutions/lanschool-air) (Lenovo) | Big vendor | [$349.99 / 50 devices / year](https://www.lenovo.com/us/en/p/accessories-and-software/software/education-and-reference-downloads/4l41q30850) ≈ $7/device/yr | Price, self-host, local language |
| [NetSupport School](https://www.netsupportsoftware.com/classroom-management-teaching-platform/) / [classroom.cloud](https://classroom.cloud/pricing/) | Big, active marketing | ~£5/device/yr ([third-party estimate](https://pricingnow.com/question/netsupport-school-pricing/), unverified) | Price, open source, AI |
| GoGuardian / Securly / Lightspeed | US Chromebook market | Quote only | Not our market (Windows labs, CIS) |
| [RustDesk](https://rustdesk.com/pricing/) | Growing fast in remote support | Pro ≈ $1/user + $0.10/device/month | Classroom features; they don't do lock/exam/blocking |

**Most important competitor data we don't have yet:** what client #1's school uses *today*. Ask on day
one. That product is the one we take the client from.

### 3b. The wedge vs Veyon — AI + Rust + design (founder's direction, 2026-09-16)

Veyon exists, is free, and is mature — so we do **not** win on the checklist of things it already does
(watch, control, lock, broadcast). We win on the three things it structurally *can't* copy cheaply, and
we say so plainly in every pitch:

1. **Works beyond the LAN, by ID.** Veyon is LAN-bound (VNC + a master on the same subnet); ours dials
   by device id over iroh, so a teacher reaches a lab from home or across sites with no VPN or port
   forwarding. This is the single biggest concrete gap and stays our headline (BUSINESS.md §1).
2. **AI nobody in this category ships.** The differentiators to build first, chosen because they are
   novel, demoable in 30 seconds, and privacy-defensible (run on-device where possible, per D16):
   - **Off-task / attention report** — classify each student's foreground app/site as on- or off-task
     and give the teacher a live "who's drifting" view plus an end-of-lesson summary. The flagship demo.
   - **Screen-safety flagging** — detect explicit/harmful content on a student screen and alert the
     teacher. Directly answers the "student sets a nudity wallpaper / opens something" worry and is a
     duty-of-care selling point for schools, not a gimmick.
   - **Natural-language control** — "lock everyone except row 3", "open the exam on all PCs" — a typed
     command the Console turns into the existing typed actions. Cheap to build on top of the action
     enum, and it *looks* like the future in a demo.
3. **Design and onboarding as the product.** Veyon is Qt, dated, and has no first-run guidance; being
   Svelte + modern + a built-in tutorial is itself the reason a teacher chooses us — the founder's
   Notion-over-Obsidian point: a proprietary-backed tool with great UX beats a free one that is hard to
   learn. Treat "a teacher succeeds in 5 minutes with no manual" as a release requirement, not polish.

**Rust backend** is a supporting proof point (fast on weak school PCs, low RAM, one static binary, no
runtime to install), not a headline — teachers don't buy a language. Use it in the technical/IT-buyer
conversation, not the teacher pitch.

**Build order this implies:** exam/lockdown mode (table stakes we still lack) → the off-task report
(flagship AI) → screen-safety flagging → natural-language control. Defer URL filtering and file
collect until a pilot actually asks; they are slow and not differentiators.

## 4. Client #0 — the founder (Phase 1, milestone M1)

Use it every day on your own 2+ PCs: view, control, lock yourself out, block your own games. If
it annoys you, it will annoy a teacher 10×. Don't contact client #1 about the pilot until M1 has held
for one week.

## 5. Client #1 — the teacher you know

| When | Action | Done when |
|------|--------|-----------|
| Phase 0, week 1 | 20-min call: film/list their lab (PC count, Windows versions, hardware, internet, current tool, what students actually do). **Don't pitch — ask.** | Answers are written into AGENTS.md §7 |
| Phase 1 end | Show M1 on your own PCs (screen recording or visit). Ask: "Would you run this on 5–10 PCs for 2 weeks?" | A pilot date is agreed |
| ≈ Month 3 | **Pilot v0** on ≤ 10 PCs with steps 2.1–2.5 (view, control, lock, power, blocking). You install it yourself, on site. | Used in ≥ 5 real lessons |
| M2 | Full free tier; daily 3-question feedback | Teacher says they'd be upset if you removed it |
| After M2 | Ask for 3 things: a quote for the landing page, an intro to 2 colleagues, 10 min with the principal | Case study + 2 warm leads |

It's free, by hand, and on site. That's the cheapest way to learn whether anyone needs it. If the
teacher stops using it after 2 weeks, it's a product problem, not a marketing one. Fix it before
looking for client #2.

## 6. Clients 2–10 — copy the ones already reaching teachers

1. **Warm referrals** from client #1: same school, then the same district's informatics teachers.
2. **Where teachers actually gather.** Ask client #1 which Telegram/VK/WhatsApp groups of informatics
   teachers they read. Post the demo video there (§8). Don't guess the channels — ask.
3. **Take clients from Veyon / NetOp users:** "Same idea, works over the internet, keeps working
   offline, has exam mode, and speaks your language."
4. **The district IT specialist:** one person who can approve many schools. Get to them through
   client #1's principal.
5. Paid ads come **only after 10 clients** (money scales a flow, it doesn't start one).

## 7. Pricing proposal (country-adjusted later; AI priced only after it's measured)

| Plan | Price | Notes |
|------|-------|-------|
| **Free** | $0, ≤ 10 devices | Every free-tier feature from the brief. Self-host or hosted Hub. |
| **Classroom** (education) | List **$99/room/year** (≤ 40 devices ≈ $2.5/device/yr). CIS regional price **~$39–59/room/year**. | Undercuts LanSchool (~$7) and NetSupport (~£5) per device. **Check your country's small-purchase / single-supplier procurement limit and keep one room's price under it**, so a principal can buy without a tender. The first semester is free for pilot schools. |
| **School / District** | Quote | Unlimited rooms, self-hosted Hub, priority support, training. |
| **Business** | **$2/device/month**, $1.60 annual; ≥ 500 devices = quote | Your number holds up: above RustDesk Pro (no control/lock features), well below employee-monitoring suites. |
| **AI add-on** | BYOK included in any paid plan. Hosted AI priced at **≥ 3× measured cost per student-hour** (Phase 6.3). | Never sell the "random free HuggingFace model". Paying schools need stable quality. |

## 8. Marketing runs ahead of the product — concrete tasks (budget $0)

- [ ] **Phase 0:** a one-page landing site (Codeberg/GitHub Pages) in Russian + English: the 3
  headlines, the founder's opinion, the "coming" feature list, and a waitlist (email or Telegram).
- [ ] **Phase 0:** a Telegram channel. **Rule: every shipped feature gets a post** (a 10-second GIF +
  one sentence), mirrored from the GitHub release notes.
- [ ] **Phase 1 end — 45-second demo video** (phone + OBS, free). Shot list:
  1. (3 s) Over the teacher's shoulder: a student alt-tabs into a game. *Frustration.*
  2. (7 s) The Console opens: a grid of 10 live screens.
  3. (5 s) One click → the game closes on the student's screen.
  4. (7 s) "Lock all" → every screen shows the exam lock with the teacher's message.
  5. (8 s) "Collect work" → a folder with every student's files appears on the teacher's PC.
  6. (5 s) Internet cable unplugged → blocking still works.
  7. (10 s) End card: *"Free for up to 10 PCs. Open source. Built for teachers."*
- [ ] **Monthly:** re-check what Veyon/NetSupport/LanSchool publish and who is advertising to teachers
  in your country right now. Copy what works for them.

## 9. Three-month hypotheses (each with a fast test anyone can run)

| # | Hypothesis | Test | Pass |
|---|------------|------|------|
| H1 | Teachers want this | Post the demo in 5 teacher groups | ≥ 30 waitlist sign-ups in 14 days |
| H2 | A principal will pay for a Classroom plan | Client #1 asks 5 principals: "Would you pay ~$X/year per room?" | ≥ 2 say yes at the CIS price |
| H3 | "Collect + wipe" is what sets us apart | Note which feature pilot teachers mention first | It's in the top 2 |
| H4 | Offline enforcement matters | Count internet outages during the pilot | ≥ 1 outage where blocking kept working |

**Stop-list:** don't trust Product Hunt, "top-10 classroom software 2026" listicles, influencer posts or
reports older than 6 months. Use them as ads to read, never as data.
