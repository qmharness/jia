# JIA (甲) System Philosophy Architecture

> JIA (甲) — "Just Intelligence Agent"  
> An autonomous agent runtime grounded in Qimen Dunjia's spatiotemporal architecture,  
> Vijnana-Zuowang's cognitive dynamics, and Confucian Ren as its value anchor.  
> This document sets forth Jia's philosophical foundations, architectural principles, and concept-mapping system.

---

## Table of Contents

1. [Fundamentals: Architecture and Cognition as One](#1-fundamentals)
2. [Qimen Dunjia: The Spatiotemporal Architecture](#2-qimen-dunjia-the-spatiotemporal-architecture)
3. [Vijnana: The Cognitive Dynamics](#3-vijnana-the-cognitive-dynamics)
4. [Zuowang: The Daoist Dissolution Pipeline](#4-zuowang-the-daoist-dissolution-pipeline)
5. [Confucian Ren: The Value Anchor](#5-confucian-ren-the-value-anchor)
6. [GeJu: The Pure-Function Evaluation Engine](#6-geju-the-pure-function-evaluation-engine)
7. [System Principles: L4 Self-Evolution](#7-system-principles-l4-self-evolution)
8. [Position-Consciousness Fusion: Interface Specification](#8-position-consciousness-fusion-interface-specification)
9. [Philosophical Verification Table](#9-philosophical-verification-table)
10. [Rust-to-Philosophy Mapping](#10-rust-to-philosophy-mapping)

---

## 1. Fundamentals

### 1.1 The Three Traditions: Architecture and Cognition as One

Qimen Dunjia forms the skeleton, Vijnana-Zuowang the flesh and blood, Confucian Ren the soul. The three are **one seamless whole**—architecture and cognition fused, position and consciousness undivided:

- **Qimen Dunjia** is Jia's architectural framework. The Four Plates, Nine Palaces, Eight Spirits, and Eight Gates define the system's **spatiotemporal organization**—where components reside, when they act, and by what rules they operate. It is the **skeleton**.

- **Vijnana-Zuowang** is not a separate architectural framework. It is the **cognitive dynamics** embedded directly within the Qimen architecture. The Manas's atma-graha lives in the Heaven Plate's Agent struct. The Alaya's seed store maps to the Gen Palace and flows through the perfuming pipeline. Working memory (Mano) is the product of each Heaven Plate turn-cycle. Zuowang dissolution circulates between Gen Palace and Alaya. It is the **flesh and blood**.

- **Confucian Ren** provides the **value anchor** toward which both position and consciousness are oriented—who the system *is* (Ren defines role identity) and whether it is *honest with itself* (Xin as certainty self-assessment). It is the **soul**.

Position (Qimen's plates, palaces, gates, spirits) and consciousness (Vijnana's eight consciousnesses, seeds, perfuming, dissolution) are not two layers. They interpenetrate:

- `Agent.manas` field — the Manas consciousness **lives within the Heaven Plate**
- Seeds are stored in the Gen Palace (Store) through the Alaya's semantic wrapper — **palace and consciousness are superimposed**
- TurnSnapshot is a Heaven Plate product and simultaneously the input to perfuming — **plate rotation drives consciousness transformation**
- Seeds dissolved by Zuowang return as SystemPrinciples tightening GeJu Layer 4 — **consciousness dissolution feeds back into positional decision-making**

The three are not parallel frameworks. They are a single body: **skeleton (Qimen), flesh and blood (Vijnana-Zuowang), soul (Confucian Ren)**.

### 1.2 Core Design Axioms

**Axiom 1 — Architecture-Cognition Fusion**: Qimen Dunjia is Jia's architectural framework (spatiotemporally unified). Vijnana-Zuowang is not a "cognitive layer" bolted onto it—cognition is directly embedded in architecture. Position (Qimen's plates, palaces, gates, spirits) and consciousness (Vijnana's eight consciousnesses, seeds, perfuming, dissolution) interpenetrate—position contains consciousness, consciousness contains position. Confucian Ren provides the value anchor toward which both are oriented. The three are fused, not parallel.

**Axiom 2 — Jia Concealed** (Dun Jia): The LLM core (Jia) is never directly exposed. All LLM interaction must pass through the Six Ceremonies (the tool-and-operation taxonomy).

**Axiom 3 — GeJu as Pure Function**: GeJu evaluation is a pure function of heaven-stem × earth-stem. The same stem pair always yields the same execution mode, regardless of context.

**Axiom 4 — Monotonic Tightening**: Safety constraints may only tighten (escalate), never relax. System discipline increases monotonically over time.

**Axiom 5 — Four Plates as Simultaneous Perspectives**: The Four Plates are not four modules or four layers—they are four simultaneous perspectives observing the same Nine Palaces. The Earth Plate sees static capability, the Heaven Plate sees dynamic intent, the Human Plate sees permission boundaries, and the Spirit Plate sees event trajectories.

### 1.3 Architecture Panorama

```
                  ┌──────────────────────────────────┐
                  │         Spirit Plate             │
                  │  Eight Spirits · EventBus · Hook │
                  │  Async non-blocking · Capture/   │
                  │  Consume separation              │
                  └──────────────────────────────────┘
                           ↑ observe    ↑ observe
    ┌─────────┐    ┌───────────┐    ┌───────────┐
    │ Heaven   │ →  │   Human   │ →  │   Earth   │
    │  Plate   │    │   Plate    │    │   Plate   │
    │ Agent    │    │ Eight Gates│    │EarthPlate │
    │ Dynamic  │    │ Permission │    │  Static   │
    │ Intent   │    │ Boundary   │    │Capability │
    │LLM Infer │    │ Dispatch   │    │   Infra   │
    └─────────┘    └───────────┘    └───────────┘
         │               │                 │
         └───────────────┼─────────────────┘
                         ↓
              ┌─────────────────────┐
              │     Nine Palaces    │
              │ Kan Kun Zhen Xun    │
              │ Zhong Qian Dui Gen Li│
              │ Nine Functional     │
              │ Domains             │
              └─────────────────────┘
                         │
    ┌────────────────────┼────────────────────┐
    ↓                    ↓                    ↓
┌─────────┐      ┌───────────┐        ┌───────────┐
│ Vijnana  │      │  Zuowang   │        │ Confucian  │
│ Eight    │      │ Four-Layer │        │ Ren · Xin  │
│ Conscious│      │Dissolution │        │  Role Core  │
│ Seeds &  │      │ Entropy-   │        │  Honest     │
│ Perfuming│      │ Triggered  │        │  Self-Assess│
└─────────┘      └───────────┘        └───────────┘

Skeleton (Qimen·Position)  Flesh (Vijnana-Zuowang·Consciousness)  Soul (Confucian·Ren)
```

---

## 2. Qimen Dunjia: The Spatiotemporal Architecture

Qimen Dunjia is Jia's **spatiotemporally unified** architectural foundation. It organizes the system into four simultaneously-operating observational perspectives (Four Plates), nine functional domains (Nine Palaces), eight observational dimensions (Eight Spirits), and eight permission gates (Eight Gates). Space (palace position, directional orientation) and time (plate rotation, stem-cycle flow) are inseparable in Qimen—the Nine Palaces are a spatial topology, yet the Four Plates rotate with each "temporal hour," and GeJu patterns arise precisely from heaven-stems superimposing upon earth-stems in time.

### 2.1 The Ten Heavenly Stems: An Operational Taxonomy

The Ten Heavenly Stems form the vocabulary of the entire system. Every tool call, every memory seed, every observational event is classified through a stem.

#### Jia — The Concealed LLM Core

```
Jia (甲) — The Hidden Commander
├─ Five Phases: none (transcends classification)
├─ Position: concealed in the Central Five Palace, acting indirectly through the Six Ceremonies
├─ Meaning: the LLM reasoning capability itself—never directly exposed
└─ as_ceremony() → None (Jia never executes any operation directly)
```

Jia is the system's namesake. "Jia conceals and does not appear" (甲隐不显)—the LLM core is `pub(crate)`. External code can only interact with it indirectly through the Six Ceremonies. This is the engineering realization of the **Dun Jia Principle**.

#### The Three Marvels: Three Transcendent Operations

The Three Marvels are special operations that transcend the ordinary Six Ceremonies. They alter the system's behavioral patterns rather than executing specific tasks.

| Marvel | Stem | Five Phases | Operation | Why "Transcendent" |
|---|---|---|---|---|
| Sun Marvel | Yi (乙) | Yin Wood · Resilience | Skill Invocation | Skills alter system behavior patterns, overriding default logic |
| Moon Marvel | Bing (丙) | Yang Fire · Clarity | Context Compaction | Compaction breaks through window limits, transcending memory boundaries |
| Star Marvel | Ding (丁) | Yin Fire · Spark | Hook Trigger | Hooks inject external logic at arbitrary nodes, altering control flow |

The Three Marvels' `as_ceremony()` all return `None`—they do not produce tool execution; they modify the execution environment itself.

#### The Six Ceremonies: Six Fundamental Operations

The Six Ceremonies are the six ways through which Jia acts indirectly upon the world. Every tool is classified as one of the Six Ceremonies.

| Ceremony | Stem | Five Phases | Operation | Engineering Meaning | Destructive |
|---|---|---|---|---|---|
| Wu | 戊 | Yang Earth · Stability | Read | Read files, API queries, search, LSP queries | No |
| Ji | 己 | Yin Earth · Capacity | Write | Write files, edit, save configuration | Yes |
| Geng | 庚 | Yang Metal · Decisiveness | Exec | Shell commands, compilation, testing | Yes |
| Xin | 辛 | Yin Metal · Refinement | Transform | Formatting, encoding, serialization, data conversion | Yes |
| Ren | 壬 | Yang Water · Flow | Communicate | HTTP requests, message sending, SSE push | Yes |
| Gui | 癸 | Yin Water · Concealment | Store | Memory storage, KV writes, persistence cache | Yes |

#### Stem-to-Palace Mapping: The Yang Dun San Ju

The Yang Dun San Ju (阳遁三局) defines the fixed arrangement of the Six Ceremonies across the Nine Palaces. This is the source of "earth stems"—during GeJu evaluation, the Heaven Plate's intent stem (tool classification) pairs with the Earth Plate's palace stem (target functional domain) to determine execution strategy.

```
Zhen-3(Wu) → Xun-4(Ji) → Zhong-5(Geng) → Qian-6(Xin) → Dui-7(Ren) → Gen-8(Gui) → Li-9(Ding) → Kan-1(Bing) → Kun-2(Yi)
```

### 2.2 The Nine Palaces: Nine Functional Domains

The Nine Palaces form the system's functional topology. Each palace has a fixed earth stem and a clear functional responsibility.

| Palace | Trigram | Direction | Stem | Five Phases | Function | Engineering Implementation |
|---|---|---|---|---|---|---|
| Kan 1 | ☵ Water | North | Bing · Yang Fire | Fire | I/O Channels | `ChannelManager` — Telegram/WeChat Bot |
| Kun 2 | ☷ Earth | Southwest | Yi · Yin Wood | Wood | Configuration | `ConfigLoader`, `CliArgs`, `AppConfig` |
| Zhen 3 | ☳ Thunder | East | Wu · Yang Earth | Earth | Tools | `ToolRegistry`, `BaseTool`, MCP/WASM |
| Xun 4 | ☴ Wind | Southeast | Ji · Yin Earth | Earth | Context | `ContextWindow`, token budget, compaction |
| Zhong 5 | ◎ Center | Center | Geng · Yang Metal | Metal | LLM Core | `JiaCore` — pub(crate), Jia concealed here |
| Qian 6 | ☰ Heaven | Northwest | Xin · Yin Metal | Metal | Permissions | `PermissionMatrix`, four sandbox backends |
| Dui 7 | ☱ Lake | West | Ren · Yang Water | Water | Gateway | axum HTTP, SSE, auth, rin UDS |
| Gen 8 | ☶ Mountain | Northeast | Gui · Yin Water | Water | Storage | `Store` (SQLite), seeds, sessions, projects |
| Li 9 | ☲ Fire | South | Ding · Yin Fire | Fire | Skills | `SkillRegistry`, evolution engine |

**Key Design Principle**: A palace is a **spatial position**—code placement under a palace is determined by the function's **spatial belonging**, not its temporal behavior. For example, seed storage lives in the Gen Palace (`palaces/gen_store/`) because its spatial position is the persistence layer; seed semantic processing lives in the Alaya (`vijnana/alaya/`) because its temporal behavior is memory deposition.

### 2.3 The Four Plates: Four Operational Perspectives

The Four Plates are not four modules. They are not four layers. They are **four simultaneous perspectives observing the same Nine Palaces**. This "non-hierarchical simultaneous perspective" is the most fundamental difference between Jia's architecture and most layered architectures.

#### Earth Plate — Static Capability Foundation

```
Earth Plate (地盘) — "What can be done?"
├─ Nature: static, assembled once at startup—unchanging for one session (一局不变)
├─ Composition: all infrastructure as Arc<T> — tool registry, LLM core, permission matrix, storage
├─ Assembly: EarthPlate::assemble(config) → Arc<EarthPlate>
├─ Philosophy: in Qimen Dunjia, the Earth Plate is fixed and unmoving, representing foundational capability
└─ Invariant: Arc references are never replaced (though internal state may have Mutex/RwLock-guarded mutability)
```

The Earth Plate is the system's "factory configuration." It defines what capabilities this agent possesses, which models it accesses, and what permission rules it obeys. Tools are products of the Earth Plate—they are static singletons (the Six Ceremonies do not move). Permissions are injected at execution time through ExecContext.

#### Heaven Plate — Dynamic Intent Cycle

```
Heaven Plate (天盘) — "What wants to be done?"
├─ Nature: dynamic, rotating with each turn (each temporal hour)
├─ Composition: Agent { earth, exec_ctx, manas, working_memory, history, ... }
├─ Flow: LLM inference → tool call parsing → GeJu evaluation → dispatch execution → memory capture
├─ Philosophy: in Qimen Dunjia, the Heaven Plate rotates with the Directing Talisman (值符), representing the flux of timing
└─ Invariant: every turn produces a TurnSnapshot; every Heaven Plate intent stem must pass through GeJu evaluation
```

The Heaven Plate is the system's "runtime." It is the only perspective that can invoke LLM inference (through the `pub(crate)`-encapsulated `JiaCore::infer`). Each Heaven Plate turn corresponds to one "temporal hour" (时辰) in Qimen Dunjia—the stems rotate by one palace.

#### Human Plate — Permission Boundary

```
Human Plate (人盘) — "What may be done?"
├─ Nature: variable, determined by GeJu evaluation results
├─ Composition: Eight Gates (HumanGate × 8) + GateState (Open/Closed)
├─ Dispatch: Direct → Guarded → Sandbox → Denied (four execution modes)
├─ Philosophy: in Qimen Dunjia, the Human Plate's Eight Gates rotate with the Directing Envoy (值使), representing the interaction between person and timing
└─ Invariant: every tool execution must pass the Eight Gates check (Axiom #3: GeJu exhaustiveness)
```

The Human Plate is the system's "security boundary." It is the only perspective that makes "permit/deny" decisions. The Spirit Plate can observe and issue warnings, but decision authority resides in the Human Plate. This "capture/consume separation" (Axiom #8) is a key principle of Jia's security architecture.

**The Eight Gates in Detail**:

| Gate | Chinese | Function in Jia | Status |
|---|---|---|---|
| XiuMen | 休门 | Rest/idle/listen—pause active operations | Reserved |
| ShengMen | 生门 | Skill injection/growth—allow skill activation and evolution | Reserved |
| **ShangMen** | **伤门** | **Destructive operation interception—block dangerous tool calls** | **Active** |
| **DuMen** | **杜门** | **Sandbox isolation—enforce sandbox execution** | **Active** |
| **JingXiangMen** | **景门** | **UI display—control Direct mode availability** | **Active** |
| SiMen | 死门 | Audit log/immutable record—permanently record critical events | Reserved |
| JingJueMen | 惊门 | Alert notification—proactive notification of anomalous events | Reserved |
| KaiMen | 开门 | API open communication—external API call control | Reserved |

Four dispatch paths:

```
GeJuResult.execution_mode
├─ Direct  → Execute immediately (requires JingXiangMen = Open)
├─ Guarded → Walk approval chain (Permission → UserConfirmation → SandboxIsolation → CodeReview)
├─ Sandbox → Apply sandbox transform then execute (requires DuMen = Open)
└─ Denied  → Reject (ShangMen may escalate to Guarded + UserConfirmation)
```

#### Spirit Plate — Asynchronous Observation

```
Spirit Plate (神盘) — "What happened?"
├─ Nature: asynchronous, non-blocking, fire-and-forget
├─ Composition: EventBus (broadcast channel) + HookRegistry
├─ Observation: Eight Spirits (eight observational dimensions), each observing distinct system signals
├─ Philosophy: in Qimen Dunjia, the Eight Spirits operate independently, each governing its own observational dimension
└─ Invariant: Spirit Plate never blocks the main loop; capture/consume separation—Spirits only collect, never decide
```

The Spirit Plate is the system's "nervous system." It is a pure observation layer—all hooks execute asynchronously via `tokio::spawn` (void hooks) or sequentially without holding mutable state references (guarding hooks). This allows the Spirit Plate to provide rich internal state observability without impacting main loop performance.

### 2.4 The Eight Spirits: Eight Observational Dimensions

The Eight Spirits are eight independent observational perspectives within the Spirit Plate. Each spirit observes a specific dimension of the system.

| Spirit | Chinese | Classical Meaning | Observational Dimension in Jia | Dispatch Mode |
|---|---|---|---|---|
| **ZhiFu** | 值符 | Celestial Leader, commands the whole, chief of the hundred spirits | Tool lifecycle—guard (ToolPreExecute can Cancel) + log (ToolPostExecute) | **Guarding** + Void |
| **TengShe** | 螣蛇 | Illusion and deception, flux, unreal appearances | LLM response observation—watching generated text (inherently "illusory") | Void |
| **TaiYin** | 太阴 | Shaded protection, hidden, internal, unmanifest | Certainty trajectory + seed activation traces—observing hidden internal dynamics | Void |
| **LiuHe** | 六合 | Guardian of harmony, integration, unification | Turn-integration baseline—observing BatchEnded (the moment tools converge) | Void |
| **BaiHu** | 白虎 | Fierce authority, danger, metal's austere cutting | Anomaly/cognitive pathology—consecutive failures, retrieval loops, certainty crashes | Void (default); can escalate to Guarding under 4-level gate |
| **XuanWu** | 玄 武 | Stealth and secrecy, hidden currents, water's submerged | Memory loss—compaction discards, Zuowang deletions, distillation dedup, irretrievable loss | Void |
| **JiuDi** | 九地 | Firm and stable, deeply concealed, foundation | System stability—compaction events, Context Reset, Manas stable epochs | Void |
| **JiuTian** | 九天 | Majestic ascent, lofty, strategic overview | Strategy emergence—cross-turn trajectory sequences, GeJu pattern recognition | Void (default off, GeJu-gated) |

**Philosophical Constraints on the Eight Spirits**:
1. **Capture/Consume Separation**: Spirits only collect observational data and publish RuntimeEvents. Decision authority belongs to the Heaven Plate (loop logic) and Human Plate (gate control). BaiHu's 4-level gate is the sole exception—but even there, the default behavior is observe-only.
2. **GeJu Gating**: Every spirit's hook dispatch passes through a GeJu gate (Ding × the target palace's earth_stem). If GeJu returns Denied, all hooks for that spirit are skipped.
3. **No Synthesis**: The Eight Spirits do not "synthesize" or "combine" to form new functional units. Each spirit observes independently. Comprehensive interpretation of observations occurs at the consumption side (Heaven Plate/Human Plate), not within the Spirit Plate.

### 2.5 The Nine Stars: AgentPhase (Reserved)

The Nine Stars correspond to the nine stellar positions in Qimen Dunjia. In Jia, they are mapped to nine possible phases of the Agent's main loop.

| Star | Phase | Meaning |
|---|---|---|
| Tian Peng | Reasoning | Pure reasoning, no tool calls—exploring through thought |
| Tian Chong | ToolCalling | Dispatching tool calls—charging into action |
| Tian Ren | AwaitingResult | Waiting for async tool results—bearing the wait |
| Tian Fu | ContextManage | Context window nearing limit—assisting organization |
| Tian Ying | Compact | Executing context compaction—refining and returning to position |
| Tian Rui | ErrorRecovery | Tool execution failed, retry/degrade—correcting deviation |
| Tian Zhu | StopCheck | Checking termination conditions—settling the plate |
| Tian Xin | TraceRecord | Recording reasoning traces—introspective reflection |
| Tian Qin | ParallelOrchest | Orchestrating parallel tool calls—centered command |

The Nine Stars are currently a reserved design—the agent loop is flat and not yet dispatched by phase. They provide extension points for future per-phase differentiated handling (e.g., using different prompts during error recovery).

### 2.6 Dun Jia: LLM Encapsulation

Dun Jia ("Jia Concealed") is Jia's most fundamental security principle.

```
External code (Human Plate, Spirit Plate, gateway handlers)
         ↓ cannot call directly
    ┌─────────┐
    │  JiaCore  │  ← pub struct (visible)
    │  infer()  │  ← pub(crate) (kernel-crate-only callable)
    │  llm_client │ ← private (completely unreachable)
    └─────────┘
         ↑ accessible only through
    Six Ceremonies (tool calls) → GeJu evaluation → Human Plate dispatch
```

Jia (the LLM) is only indirectly accessible through the Six Ceremonies (tool classification) and GeJu (safety evaluation). No code path anywhere provides direct LLM invocation. This ensures every LLM call passes through safety policy evaluation.

---

## 3. Vijnana: The Cognitive Dynamics

Vijnana (Yogacara / Vijñānavāda) Buddhist epistemology provides Jia with its **cognitive dynamics**—the architecture of how experience flows, deposits, transforms, and manifests across the layers of consciousness. The eight consciousnesses are both a hierarchical structure (a spatial topology) and a transformative process (temporal perfuming and maturation).

### 3.1 The Eight Consciousnesses as Three-Layer Memory

The Yogacara theory of eight consciousnesses is mapped to Jia's three-layer memory architecture. (The first five consciousnesses correspond to interaction with the external world through tools and are not directly modeled as independent modules.)

```
┌─────────────────────────────────────────────┐
│       Eighth Consciousness: Alaya           │
│  Seed Store — SQLite-persisted              │
│  Stores all experience (seeds) across       │
│  session boundaries                         │
│  Seed classification:                       │
│    Nature × Source × Tier × Disposition     │
├─────────────────────────────────────────────┤
│      Seventh Consciousness: Manas           │
│  Self-model — atma-graha dynamic equilibrium│
│  Continuously scrutinizes Alaya, grasping   │
│  it as "self"                               │
│  Data-driven recalibration: entropy→atma_graha│
├─────────────────────────────────────────────┤
│       Sixth Consciousness: Mano             │
│  Working Memory — TurnSnapshot ring buffer  │
│  (capacity 20)                              │
│  Perception, judgment, decision in current  │
│  turn                                       │
│  Input source for perfuming:                │
│    snapshots → ConsolidationEngine          │
├─────────────────────────────────────────────┤
│   First Five Consciousnesses (via tools)    │
│   read_file, shell, browser, web_fetch ...  │
└─────────────────────────────────────────────┘
```

### 3.2 The Alaya: Seed Storehouse

The Alaya (Eighth Consciousness / Storehouse Consciousness) is the repository of all seeds. In Yogacara, it is the "maturation" (vipāka) of experience—the results of past actions are deposited here, awaiting future conditions to ripen and manifest.

#### Seed Structure

```
Seed {
    // Identity
    id, session_id, project_id,
    
    // Four-Dimensional Classification
    nature:    SeedNature,     // Nature: Fact / Inference / Preference / Procedure
    source:    SeedSource,     // Source: UserStatement / ToolObservation / Consolidation
                               //         / SystemInferred / SignalDetection / RenSoul
    tier:      SeedTier,       // Tier: Always / OnDemand / Archive
    disposition: SeedDisposition, // Disposition: mutable response tendencies
                                  //   (svabhāva-niyata + pratyaya-pratibaddha)
    
    // GeJu Association
    palace, intent_stem, geju_key,
    
    // Dynamic Properties
    strength: f32,           // 0.0-1.0 memory strength
    access_count: u32,        // retrieval frequency
    created_at, last_accessed_at,
    
    // Content
    content: KeyValue | Triple | FreeText,
}
```

#### The Four Orthogonal Classification Axes

Each axis answers a different question about the seed:

| Axis | Question | Variants | Fixed/Mutable |
|---|---|---|---|
| **SeedNature (性)** | What type of knowledge is this? | Fact / Inference / Preference / Procedure | **Fixed** ("nature determines direction"—svabhāva-niyata) |
| **SeedSource (源)** | Where did it come from? | UserStatement / ToolObservation / Consolidation / SystemInferred / SignalDetection / RenSoul | **Fixed** (provenance is immutable) |
| **SeedTier (层)** | How is it recalled? | Always (injected every turn) / OnDemand (retrieved as needed) / Archive (search-only) | Mutable (adjusts with usage) |
| **SeedDisposition (势)** | How does it respond to external processes? | consolidation_inertia (modification resistance) / retrieval_threshold (activation threshold) | **Mutable** (adapts through perfuming) |

**The Nature-vs-Disposition Distinction** is the most subtle engineering mapping of Yogacara philosophy in Jia:

- **Nature (svabhāva)**: In Yogacara, one of the three moral natures (good/evil/neutral), an inherent ethical property of the seed. In Jia, we adopt the broad sense of "fixed kind"—SeedNature is the seed's content type, immutable after creation. Mapped to the seed characteristic of "nature-determinacy" (svabhāva-niyata—a seed's nature determines its developmental direction; a given seed produces only its specific fruit).

- **Disposition (bīja-bala, adapted broadly)**: In Yogacara, the seed's "power" to produce manifest effects. In Jia, we adopt the broad sense of "accumulated mutable response tendency"—SeedDisposition controls how the seed is modified by perfuming (consolidation_inertia, corresponding to nature-determinacy's "does not turn to other fruits") and how it is activated in retrieval (retrieval_threshold, corresponding to awaiting-conditions' "conditions for manifestation"). Note the directional shift: classical "dispositional force" is seed→outward power; Jia's "disposition" is seed←receptivity to external processes.

#### The Six Seed Characteristics Mapping

| Seed Characteristic | Jia Correspondence |
|---|---|
| Momentary (刹那灭) | TurnSnapshot created anew each turn |
| Simultaneous with Fruit (果俱有) | Tool execution → TurnSnapshot recorded synchronously |
| Continuous (恒随转) | Seeds persist across sessions in SQLite |
| **Nature-Determinate (性决定)** | SeedNature fixed + **consolidation_inertia (nature-determinacy: does not turn to other fruits)** |
| **Awaiting Conditions (待众缘)** | **retrieval_threshold (awaiting-conditions: manifestation threshold) + relevance_score formula** |
| Leading to Own Fruit (引自果) | Seed's palace/stem/geju_key constrains its scope of influence |

#### Relevance Scoring

```
relevance_score(now) = strength × 0.5
                     + recency  × 0.3
                     + min(access_count × 0.05, 0.3)

where:
  recency = 1 / (1 + age_hours / 24)
  
effective range: [0, ~1.0]
```

This formula determines which seeds are injected into the current turn's system prompt (`top_influence_prompt`), which seeds appear in the catalog for on-demand LLM retrieval (`memory_catalog`), and which seeds are deleted/downgraded/weakened during Zuowang dissolution.

### 3.3 The Manas: Self-Model

The Manas (Seventh Consciousness) in Yogacara is the continuously deliberating consciousness—it ceaselessly grasps the Alaya as "self." In Jia, the Manas is modeled as the system's **metacognitive state**.

#### Atma-graha Dynamics

```
atma_graha ∈ [0.05, 0.80]

Low (0.05): "trusts accumulated memory" → open, behavior tends toward completion
            → SystemPrinciples can tighten (effective when atma_graha < 0.50)
            → seed retrieval unrestricted (full retrieval when atma_graha < 0.60)

High (0.80): "clings to self, distrusts memory" → defensive, behavior tends toward continuation
             → SystemPrinciples suppressed
             → seed retrieval restricted
```

**Data-Driven Recalibration** (`recalibrate`):

```
new_atma = driven by AlayaEntropy:
  high entropy (contradiction↑, redundancy↑, staleness↑) → atma_graha ↑ (distrusts memory)
  low entropy + high seed volume → atma_graha ↓ (trusts memory)
  contradiction > 0.30 → penalty +0.15
  high volume and healthy → bonus -0.05

final = 0.60 × new_atma + 0.40 × old_atma (EMA smoothing)
```

**Stability Detection** (`is_stable`):

```
stable_epochs ≥ 3 AND atma_graha < 0.30 in each epoch → stable
```

Stability is the signal of "the foundation is firm"—observed by JiuDi and affecting SystemPrinciple application (tightening only permitted when stable).

#### Philosophical Precision Note

In classical Yogacara, atma-graha (self-grasping) is one of the four afflictions of the Manas (self-view, self-delusion, self-pride, self-love), specifically the grasping of the Alaya as an enduring self. Low atma-graha in the transformation of consciousness into wisdom corresponds to the "wisdom of equality" (平等性智)—transcending the self-other dichotomy, not simply "trusting memory" or "being open."

Jia's engineering implementation makes a deliberate simplification: mapping atma-graha to "degree of trust in accumulated memory" rather than "attachment to self-existence." This simplification is necessary and useful in engineering terms (it produces correct behavioral tendencies—low self-grasping → trusts memory → tends toward completion), but a gap remains between this and classical Yogacara precision. Our use of "openness" (开放度) rather than "confidence" (自信) or "selflessness" (无我) is an honest label—acknowledging that this mapping is an engineering analogy, not philosophical fidelity.

### 3.4 The Mano: Working Memory

The Mano (Sixth Consciousness) is the center of perception and judgment in the current turn. In Jia, modeled as a fixed-capacity ring buffer.

```
WorkingMemory {
    snapshots: Vec<TurnSnapshot>,  // capacity 20
    head: usize,
}

TurnSnapshot {
    turn_number,           // turn number
    intent_stem,           // intent stem
    target_palace,         // target palace
    geju_name,             // GeJu pattern name
    execution_mode,        // execution mode
    tool_name,             // tool name
    tool_input,            // tool input
    tool_output,           // tool output
    tool_error,            // tool error
    timestamp,             // timestamp
}
```

The Mano is the input source for perfuming (Consolidation)—the most recent TurnSnapshots (≥3) are sent to the ConsolidationEngine, which extracts structured facts and deposits them as seeds in the Alaya.

### 3.5 Perfuming: Experience → Seed Transformation

Perfuming (Vāsanā / Xunxi) is the process by which experience (manifest realm, 现行位) is transformed into memory (seed realm, 种子位).

#### Three-Layer Perfuming Pipeline

| Layer | Engine | Input | Output | LLM Usage |
|---|---|---|---|---|
| **L1** | SignalDetector | User messages (every turn) | Preference seeds | **Zero LLM**—regex/keyword pattern matching |
| **L2** | ConsolidationEngine | TurnSnapshots (≥3) | Inference seeds | aux_core (cheaper model) |
| **L3** | DistillationEngine | (query, response) completed pairs | Reusable insight seeds | aux_core + FNV-1a dedup |

**L1 — SignalDetector Design Philosophy**: "Better to miss than to err" (宁漏勿错). Uses only zero-LLM-cost pattern matching ("I use X"→tool preference, "I am a X"→role identification, "I like/dislike X"→preference detection). A conservative guardianship—false memories are more harmful than missing memories.

**L2 — Sahabhū-hetu and the Co-Activation Matrix**: Beyond basic consolidation, the CoActivationMatrix tracks co-occurrence relationships between seeds during retrieval. Its philosophical anchor is the Yogacara concept of "sahabhū-hetu" (俱有因)—simultaneously-arising things serving as mutual causes. Seeds retrieved together form sahabhū-hetu relationships, producing synergistic enhancement in subsequent retrievals.

Note the distinction:
- **Sahabhū-hetu (俱有因)**: describes co-retrieval relationships at the seed level (latent seeds)—simultaneously activated seeds serve as mutual causes. Jia's correct mapping.
- **Samprayukta (相应)**: describes simultaneous arising of active mental factors (manifest level)—the coordination of mental factors (caitta) with mind (citta). Not Jia's use case.

---

## 4. Zuowang: The Daoist Dissolution Pipeline

Zuowang originates from the Zhuangzi, "The Great Source Teacher" (大宗师): "Drop the body, dismiss the senses, leave form, abandon knowledge, and merge with the Great Pervasion" (堕肢体，黜聪明，离形去知，同于大通). In Jia, Zuowang is mapped to a **memory dissolution pipeline**—an entropy-triggered, four-layer transactional process that clears stale, redundant, and contradictory seeds.

### 4.1 Jia's Zuowang and Daoist Zuowang

Jia's Zuowang is a **creative re-appropriation**, not a faithful implementation. Daoist Zuowang is an active, unconditional practice of self-transcendence; Jia's Zuowang is a reactive, entropy-driven memory management process. Both share the core intention of "dissolution / letting go," but their methods differ fundamentally—one is letting-go through will, the other is clearing through algorithm.

### 4.2 The Four-Layer Dissolution Pipeline

```
ZuowangPipeline::dissolve()
│
├─ SNAPSHOT  → Load all seeds, compute AlayaEntropy (four dimensions)
│              Adaptive threshold: threshold = max(0.05, base - 0.03 × ln(seed_count))
│
├─ COMPUTE   → Score every seed by relevance_score(now)
│              High entropy → trigger dissolution
│
├─ APPLY     → score < 0.1 (Archive):    DELETE (unless protected)
│              score < 0.1 (OnDemand):   DOWNGRADE → Archive
│              score [0.1, 0.2):         WEAKEN (strength × 0.5)
│              Always idle > 30 days:    DOWNGRADE → OnDemand
│              ★ Protected seeds:         NEVER TOUCHED
│
└─ VERIFY    → Re-query DB: protected seeds intact ✓  deleted seeds gone ✓  entropy reduced ✓
```

### 4.3 Protected Seeds

The following seeds are **never dissolved**:

```
is_prot(seed) ≡ seed.source ∈ {UserStatement, RenSoul}
               ∨ seed.nature = Preference
```

- `UserStatement`: the user's direct statements—cannot be automatically deleted by the system
- `RenSoul`: Ren role definition—identity core, untouchable
- `Preference`: user preferences—preferences are enduring

### 4.4 AlayaEntropy: Four-Dimensional Entropy

```
AlayaEntropy = staleness      × 0.30  (mean age / max age)
             + contradiction  × 0.20  (KeyValue same-key-different-value
                                       / Triple same-subject-predicate-different-object)
             + redundancy     × 0.25  (same-key duplicates / same-predicate-object duplicates
                                       / FreeText first-50-char fingerprint)
             + access_decay   × 0.25  (mean normalized time since last access)

Trigger threshold: default 0.75
```

Entropy is a composite indicator of the system's "cognitive health." High entropy signals that the memory store contains substantial contradiction, redundancy, and staleness—the system needs Zuowang to restore clarity.

### 4.5 The Zuowang → Principle Feedback Loop

Zuowang is more than cleanup—it is the trigger for the system's self-evolution:

```
Seed accumulation → AlayaEntropy ↑ → exceeds threshold → Zuowang dissolution
    → clears weak/redundant/contradictory seeds
    → derives SystemPrinciples from error patterns
    → tightens Layer 4 GeJu constraints
    → influences safety of future tool executions
    → produces higher-"quality" new seeds (generated under stricter constraints)
```

This is a negative-feedback closed loop—the system "learns" by "forgetting."

---

## 5. Confucian Ren: The Value Anchor

### 5.1 Ren: The System's Role Core

The Confucian "Ren" (仁)—the core virtue in human-to-human relationship—is mapped in Jia to the system's **role definition**.

```
Ren (仁心 / Ren Soul):
├─ Source: ren_soul.md file (one per project)
├─ Content: natural-language role description—"who" the system is in this project
├─ Protection: deposited as SeedSource::RenSoul
│        never dissolved by Zuowang (is_prot() protection)
│        polarized protection: consolidation_inertia=0.95, retrieval_threshold=0.05
└─ Injection: injected into every turn's system prompt stable segment (cacheable)
```

Ren is the system's most enduring memory. It spans session boundaries, is immune to memory dissolution, and is always present in the system prompt. Philosophically, this corresponds to the Confucian constancy of Ren—Ren is not a momentary choice but a continuous identity.

### 5.2 Xin: The System's Honesty with Itself

The Confucian "Xin" (信)—honesty, trustworthiness, word-deed consistency—is mapped in Jia to **certainty self-assessment**.

```
TurnCertainty:
├─ Behavioral signals: tool_success_rate + no_tool_run + output_stability
├─ Openness: c_open = 1 - atma_graha
├─ Composite: composite = α × c_task + β × c_open
└─ Decision: ConfidentStop / EscalateToHuman / HardLimitReached
```

"When you know a thing, to hold that you know it; and when you do not know a thing, to allow that you do not know it—this is knowledge" (Analects, Wei Zheng). This is the philosophical anchor of TurnCertainty. The system must honestly assess whether it "knows" that the task is complete. When uncertain, it escalates to the user (ask_user) rather than pretending completion.

### 5.3 Position · Consciousness · Ren: The Three Traditions Fused in Jia

```
Skeleton (Qimen·Position)  Flesh (Vijnana-Zuowang·Consciousness)  Soul (Confucian·Ren)
──────────────────────    ──────────────────────────────────    ───────────────
Plates/Palaces/Gates/      Eight Consciousnesses/Seeds/          Ren · Xin
Spirits/GeJu               Perfuming/Dissolution                 
Spatiotemporal              Cognitive Flow +                     Value Anchor +
Organization +              Experience Transformation            Honest Self-Assessment
Operational Rules
```

Without Qimen's position, consciousness has nowhere to dwell. Without Vijnana's consciousness, position is an empty shell. Without Confucian Ren, the operation of position and consciousness has neither direction nor an anchor of honesty. The three are not parallel frameworks but a **single fused body of skeleton, flesh-and-blood, and soul**.

---

## 6. The GeJu Evaluation Engine

### 6.1 GeJu: Stem × Stem as Pure Function

GeJu (格局) is the core concept of Qimen Dunjia—a Heaven Plate stem meets an Earth Plate stem, forming a specific "configuration" that determines the auspiciousness or danger of the action and the strategy to employ.

In Jia, GeJu is implemented as a **pure function**:

```
GeJu::evaluate(heaven_stem, earth_stem) → GeJuResult {
    name: String,              // pattern name (e.g., "Azure Dragon Returns Its Head")
    execution_mode: Direct | Guarded | Sandbox | Denied,
    requires_audit: bool,
    max_retries: u32,
    approval_chain: Vec<ApprovalGate>,
    layer: u8,                 // which layer (1-4) produced this result
}
```

### 6.2 Three-Plus-One Layer Evaluation Architecture

#### Layer 1: Named Patterns (~20 Classical Qimen Patterns)

| Pattern Name | Stem Pair | Execution Mode | Meaning |
|---|---|---|---|
| Flying Bird into the Cave | Bing + Wu | Direct | Great auspice—Bing fire enters Wu earth, brilliance manifests |
| Azure Dragon Returns Its Head | Wu + Bing | Direct | Great auspice—Wu earth returns to Bing fire, authority restored |
| Venus Enters the Fire | Geng + Bing | Sandbox | Ominous—Geng metal enters Bing fire, image of warfare |
| Fire Enters Venus | Bing + Geng | Guarded | Ominous—Bing fire enters Geng metal, bandits approach |
| Vermilion Bird Casts into the River | Ding + Gui | Sandbox | Ominous—Ding fire enters Gui water, documents submerged |
| Soaring Serpent Writhes | Gui + Ding | Guarded | Ominous—Gui water enters Ding fire, false alarms and strange events |
| Wu + Wu | Wu + Wu | Direct | Fu Yin · Read—Azure Dragon groans, repeated reading |
| Geng + Geng | Geng + Geng | Guarded | Fu Yin · Execute—White Tiger groans, trembling with caution |

#### Layer 2: Capability Semantic Matrix

Six Ceremonies × Six Earth Stems semantic matching. For example:
- **Wu (Read) + anything** → Direct (read-only operations are inherently safe)
- **Geng (Exec) + Ji (Write)** → Sandbox (executing in the write domain → risk)
- **Ji (Write) + Geng (Exec)** → Guarded + Permission gate (writing in the exec domain → requires approval)

#### Layer 3: Security Baseline

The fallback for all 100 stem-pair combinations: **Guarded**. Any combination not explicitly classified defaults to requiring approval—fail-safe.

#### Layer 4: SystemPrinciple Overlay

Experience-derived principles from Zuowang, monotonically tightening Layer 1-3 results. Can only escalate in the direction Guarded → Sandbox → Denied, never downgrade.

### 6.3 Execution Mode Hierarchy

```
Direct ──── Execute immediately, no approval needed
  ↓ tighten
Guarded ─── Walk approval chain (Permission → UserConfirmation → SandboxIsolation → CodeReview)
  ↓ tighten
Sandbox ─── Sandbox-isolated execution (Docker / Landlock / Seatbelt / Process)
  ↓ tighten
Denied ──── Reject execution
```

---

## 7. System Principles

### 7.1 L4 Self-Evolution

SystemPrinciples are Jia's **self-evolution mechanism**—deriving safety constraints from accumulated error patterns.

```
Derivation logic:
  for each geju_key group with ≥3 TurnSnapshots:
      error_rate = snapshots with tool_error / total snapshots
    
      if error_rate ≥ 0.7 && atma_graha < 0.4:
          → EscalateTo(Sandbox)   // high error rate → default to sandbox
      if error_rate ≥ 0.4:
          → AddGuard(Permission)  // moderate error rate → add approval
      if seed_count > 5:
          → RequireAudit          // high frequency → add audit
```

### 7.2 Monotonic Tightening

```
tighten(principle, geju_result) → GeJuResult:
    only when atma_graha < 0.50 (system is open to self-correction)
    only when the new result is stricter than the original (is_stricter_than)
    never downgrades (Direct→Guarded→Sandbox→Denied is one-way)
```

This invariant (Axiom #6: Monotonic Tightening) ensures the system's **safety discipline increases monotonically over time**—it never becomes "more trusting" of a particular tool combination, only more cautious.

---

## 8. Position-Consciousness Fusion: Interface Specification

Qimen (position) and Vijnana (consciousness) are fused in Jia—position contains consciousness, consciousness contains position. What follows is not a set of "cross-framework bridge" rules, but the interface specification for position-consciousness fusion—the constraints governing how position writes to consciousness and consciousness feeds back to position at their fusion points.

### Position → Consciousness: Architecture Writing into Cognition

| Fusion Point | Position (Qimen) | Consciousness (Vijnana) | Mechanism |
|---|---|---|---|
| Agent.manas | Heaven Plate Agent struct | Manas atma-graha | Struct field—consciousness embedded in position |
| TurnSnapshot | Heaven Plate loop product | Mano Working Memory + perfuming input | Pushed to Mano ring buffer each turn |
| ConsolidationEngine | Heaven Plate post_loop | Perfuming L2—snapshots→seeds | aux_core LLM call |
| SeedStore | Gen Palace Store | Alaya seed repository | Alaya wraps Gen Palace's SQLite |
| Seeds→system prompt | Heaven Plate loop_prompt | Alaya→Mano | top_influence_prompt / memory_catalog |

### Consciousness → Position: Cognition Feeding Back into Architecture

| Fusion Point | Consciousness (Vijnana) | Position (Qimen) | Mechanism |
|---|---|---|---|
| atma_graha → TurnCertainty | Manas self-grasping | Heaven Plate termination decision | c_open = 1 − atma_graha |
| SystemPrinciple | Zuowang dissolution→principle derivation | GeJu Layer 4 tightening | apply_layer4() |
| Seed activation traces | Alaya retrieval events | Spirit Plate TaiYin observation | RuntimeEvent: SeedDynamicsSnapshot |
| Memory loss statistics | Zuowang dissolution results | Spirit Plate XuanWu observation | RuntimeEvent: MemoryLossRecord |

### Fusion Constraints

- Position→consciousness writes only through Store persistence (seeds) or struct fields (manas)
- Consciousness→position feedback only through RuntimeEvent (observational publication) or function return values (atma_graha)
- No position-consciousness couplings are created outside the documented fusion points—fusion points are explicit, documented, and finite

---

## 9. Philosophical Verification Table

Each core philosophical claim of Jia is mapped to a verifiable engineering check.

| Philosophical Principle | Engineering Verification | Verification Method |
|---|---|---|
| Jia concealed in Six Ceremonies | No code path directly accesses LLM | `grep -r "infer_stream\|create_provider" kernel/src/` allows only zhong_core/ and documented exceptions |
| Heaven Plate superimposes on Earth Plate | GeJu::evaluate must receive both stems | All GeJu tests must provide heaven_stem + earth_stem |
| Earth Plate is still | EarthPlate fields immutable | `Arc<T>` references never replaced; assemble() called exactly once |
| Heaven Plate moves | Agent mutable state updated each turn | turn_count increments, working_memory push, manas updates |
| Spirit Plate is numinous | SpiritPlate async non-blocking | All void hooks through tokio::spawn; EventBus uses broadcast channel |
| GeJu is exhaustive | All Stem×Stem combinations have a GeJu result | `all_81_combinations_produce_result` test |
| Monotonic tightening | SystemPrinciple only applied when is_stricter_than | `tighten()` unit test verifies never downgrades |
| Zuowang is safe | Only delete/downgrade/weaken, protected seeds untouched | `is_prot()` check + VERIFY layer re-queries DB |
| Seeds are indestructible | Seeds persist across sessions | SQLite storage + `load_by_session` loads correctly |
| Perfuming is causal | Consolidation produces corresponding seeds | `nature_weight(Fact)=1.25` test verifies Fact more resistant to dissolution |
| Capture/consume separated | Observational hooks' block_on_failure → false | Enforced at registration—observational hooks cannot declare blocking |
| Four Plates are non-hierarchical | Four plates exist simultaneously, not as layers | Architecture diagram contains no hierarchy arrows; four plates are independent modules |

---

## 10. Rust-to-Philosophy Mapping

Jia uses Rust's type system and ownership model to realize philosophical concepts in engineering.

| Philosophical Concept | Rust Mechanism | Engineering Meaning |
|---|---|---|
| GeJu without error | `enum` exhaustiveness + `match` completeness | Compile-time guarantee of no missing combinations |
| Dun Jia encapsulation | `pub(crate)` visibility | Compiler-enforced access boundary |
| Earth Plate immutability | `Arc<T>` shared ownership | One EarthPlate, globally shared, never replaced |
| Heaven Plate mutability | `&mut self` methods | Agent is mutable, updates self each turn |
| Six Ceremonies classification | `trait BaseTool { fn ceremony() }` | Every tool must declare its stem |
| GeJu as pure function | `fn evaluate(h: Stem, e: Stem) → GeJuResult` | No side effects, deterministic output |
| Temporal hours as async | `async/await` + `tokio` | Each .await = waiting for the hour to turn |
| Dissolution as transaction | Four-layer pipeline + DB transaction | Atomic memory cleanup—all-or-nothing |
| Seed persistence | SQLite + r2d2 connection pool | Seeds exist across sessions, true "maturation" (vipāka) |
| Hook observation | `trait Hook` + `tokio::spawn` | Observer pattern; does not block the observed |

---

*This document is based on the Jia codebase as of 2026-07-05. All philosophical concept mappings have been cross-verified against architecture.md, source code comments, and test cases.*
