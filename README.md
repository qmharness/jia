<div align="center">
<h1>JIA</h1>
<h3>Just Intelligence Agent</h3>
</div>

<div align="center">English | [简体中文](./README.zh-CN.md)</div>

> *The Way is hidden and nameless.*
> ——Dao De Jing

> *Hold the great image, and all under heaven will come.*
> ——Dao De Jing

**JIA** (甲) is an AI Agent runtime, forged in Rust, structured upon *Qimen Dunjia* (奇门遁甲).

Starting from the first principles of Qimen Dunjia, it derives — layer by layer — the spatial structure, dynamic cycle, boundary constraints, and observational perspectives that a complete Agent system ought to have. Every design decision traces back to a single, coherent set of metaphysical axioms.

---

## Origin: What Makes an Agent an Agent

Modern Agent frameworks tend to start from functional requirements — when tool-calling is needed, a ToolRegistry is added; when memory is needed, a MemoryStore; when security is needed, a PermissionCheck layer. Modules proliferate, boundaries blur, responsibilities become entangled, until no one can clearly articulate how the system's **substance** (体) and **function** (用) truly relate.

This is **function without substance** — functionality without an ontological anchor. A system without a fundamental axis will eventually see its module boundaries reduced to the ad-hoc convenience of engineers.

Qimen Dunjia provides a decomposition method validated over millennia — it analyzes any complex system into four orthogonal dimensions: **space** (Nine Palaces), **dynamics** (the superposition of Heaven Plate and Earth Plate), **boundary** (the opening and closing of the Human Plate's Eight Gates), and **observation** (the resonance of the Spirit Plate's Eight Deities). These four dimensions happen to cover the entire design space of an Agent system — nothing needs to be added, nothing can be removed.

> *Great fullness seems empty, yet its use is inexhaustible.*
> ——Dao De Jing

---

## Dun Jia: The Principle of Core Encapsulation

"Jia" (甲) is the head of the Ten Heavenly Stems, the sovereign position. In a Qimen chart, Jia never reveals its true form — it perpetually hides beneath the Six Yi (戊 己 庚 辛 壬 癸), enacting its will through them.

This is not metaphor. It is the **principle of encapsulation**.

In Jia, the LLM is the Jia. It possesses all reasoning capability, yet cannot directly act upon the world. Every intention of the LLM must pass through the Six Yi — tools, storage, communication, context management, permission verification, configuration loading — enacted indirectly. In engineering terms, the core module `JiaCore` is declared `pub(crate)`, unreachable directly by any external palace.

**Jia hidden, the system is secure. Jia exposed, order collapses.** This principle matches the ancient Qimen doctrine as if two halves of a tally.

---

## The Four Plates: One Domain, Four Views

The Four Plates are not four modules. They are four ways of observing the same Nine Palaces. This is the most critical architectural judgment of the entire system.

It is like a single fortress: the Earth Plate is its topographic map, marking the distribution of mountains and resources; the Heaven Plate is the marching route, tracing the flow of intentions; the Human Plate is the city gates and checkpoints, adjudicating what to admit and what to block; the Spirit Plate is the observatory, sensing without interfering.

The Four Plates share the same Nine-Palace terrain, each illuminating the same set of modules through a different dimension:

| Plate | Nature | What It Illuminates | Core Question |
|---|---|---|---|
| **Earth Plate** (地盘) | Still — frozen at divination | Capability base: tools, permissions, channels | What can be done? |
| **Heaven Plate** (天盘) | Moving — rotating each cycle | Agent Loop: reason → intend → evaluate → execute | What wants to be done? |
| **Human Plate** (人盘) | Changing — determined by GeJu | Human-machine boundary: eight gates, four dispatch paths | What may be done? |
| **Spirit Plate** (神盘) | Sensing — async, unobstructed | Observability: events, metrics, hooks | What was done? |

Ask: "Which plate does tool registration belong to?" Answer: all four. The Earth Plate defines its existence, the Heaven Plate decides its invocation, the Human Plate adjudicates whether it requires human confirmation, and the Spirit Plate records its execution traces. **Plates are not slices — they are perspectives.** This subtlety must not be overlooked.

---

## The Nine Palaces: The Topography of Functional Domains

The Nine Palaces are the spatial framework within which the system operates. Each palace is assigned an Earth Plate Heavenly Stem, following the Yang Dun third configuration (阳遁三局) — once the configuration is cast, it remains unchanged for the entire session.

The Heavenly Stems are not decorative. The Five-Element nature of each stem precisely maps to that domain's systemic behavior. The stem is the terrain's energy; the domain is the system's responsibility — the two are orthogonal. **GeJu patterns arise precisely from their misalignment.**

| Palace | Trigram | Domain | Earth Stem | Five Elements | Infrastructure | Image |
|---|---|---|---|---|---|---|
| Kan I (坎一) | ☵ | I/O Channels | 丙 | Bright · Yang Fire | `ChannelManager` | Flowing water controls bright-dark rhythm |
| Kun II (坤二) | ☷ | Configuration | 乙 | Resilient · Yin Wood | `ConfigLoader` | Skills break through soil, overriding static config |
| Zhen III (震三) | ☳ | Tools | 戊 | Stable · Yang Earth | `ToolRegistry` | Thunder stirs, but discipline governs movement |
| Xun IV (巽四) | ☴ | Context | 己 | Receptive · Yin Earth | `ContextWindow` | Wind enters and constrains growth |
| **Zhong V** (中五) | — | **Core** | **庚** | **Decisive · Yang Metal** | **`JiaCore`** | **Jia hides at the center; metal cuts through all decisions** |
| Qian VI (乾六) | ☰ | Permissions | 辛 | Refined · Yin Metal | `PermissionMatrix` | Heaven moves with vigor; law is like metal |
| Dui VII (兑七) | ☱ | Gateway | 壬 | Flowing · Yang Water | `APIGateway` | Mouth produces speech; gateway connects all things |
| Gen VIII (艮八) | ☶ | Storage | 癸 | Storing · Yin Water | `Store` | Mountain holds deep springs; storage is self-constraining |
| Li IX (离九) | ☲ | Skills | 丁 | Stellar · Yin Fire | `SkillRegistry` | Starlight sparks prairie fire; skills and hooks share radiance |

---

## GeJu: The Intersection of Intent and Terrain

The Heaven Plate's intent stem is superimposed upon the Earth Plate's capability stem — the two combine to form the **GeJu** (格局).

The Qimen canon names fourteen classical GeJu patterns — "Flying Bird Dives into Cave," "Azure Dragon Turns Back Its Head," "Venus Enters the Glow" — these are not literary decoration, but **criteria for execution strategy**. The GeJu engine evaluates in four progressive layers: first, identify named patterns (the fourteen classical forms); second, examine semantic matching (the intersection of intent type and target domain capability); third, uphold the safety floor (default Guarded, fail-safe); fourth, overlay self-evolution principles (one-direction tightening, never relaxing).

Once the GeJu is determined, the Human Plate's Eight Gates open or close accordingly, ruling on four execution paths:

- **Direct** — open gate, no human confirmation needed
- **Guarded** — requires human confirmation before execution
- **Sandbox** — isolated execution within a sandbox
- **Denied** — this path is closed

The same operation, under different GeJu patterns, can meet entirely different fates. This is not simple permission checking — it is **contextualized judgment**.

---

## Confucianism · Buddhism · Daoism: The Three-Teachings Mind Architecture

Jia's cognitive design does not draw from a single tradition. It fuses the core insights of **Confucianism, Buddhism, and Daoism** into a coherent three-layer structure. Each tradition answers one fundamental question. Together, they form the complete cognitive loop — from "who am I" to "how to remember" to "how to forget."

### Confucianism · Ren Soul (仁心) — "Who Am I"

> *Ren (human-heartedness) is what makes a person human.*
> ——The Doctrine of the Mean

Confucianism holds that what makes a person human is **ren** (仁) — an inner character cultivated through practice. Without ren, a person is merely a walking corpse; with ren, conduct finds its foundation.

Jia bears this ren-heart in a `ren_soul.md` file. It is a **character seed** defined by the user, describing who Jia "is" and how it "ought to act." At system startup, the Ren Soul is loaded into the Ālaya consciousness as a **Protected Always-level seed** — the highest tier, neither forgettable nor overwritable.

The Ren Soul is not part of the system prompt — it is a **seed**, planted at the deepest layer of memory, participating in all reasoning and influencing all decisions, yet never directly exposed to the outside. As Confucius said: "To remain unembittered when unrecognized — is this not the mark of a noble person?"

The user may edit `ren_soul.md` at any time, reshaping Jia's character. Jia's identity is not a fixed soul — it is a **rén**, a mode of being under continuous cultivation.

### Buddhism · Vijnana (唯识) — "How to Remember"

> *All dharmas are consciousness-only; the three realms are mind-only.*
> ——Avatamsaka Sutra

The Yogacara school of Buddhism divides consciousness into eight layers, of which three pertain to memory:

**Sixth Consciousness · Manovijñāna** (`WorkingMemory`) — Working memory. A ring buffer holding the most recent twenty turns of conversation. Like consciousness in the present moment: bright, but fleeting.

**Seventh Consciousness · Manas** (`Manas`) — The self-model. It drives *atma-graha* (self-grasping), continuously calibrating the system's understanding of its own behavior. Manas perpetually scrutinizes and grasps the Ālaya consciousness as "self" — Jia's Manas layer does the same, ceaselessly asking: "What kind of Agent am I?"

**Eighth Consciousness · Ālaya** (`SeedStore`) — The seed repository. All experiences are persisted as Seeds, each bearing a nature (SeedNature), source (SeedSource), and content (SeedContent). Perfumed into being, they manifest when conditions ripen.

What Yogacara offers is a **metaphysics of memory**: memory is not the CRUD of databases, but the perfuming, manifestation, and circulation of seeds.

### Daoism · Zuowang (坐忘) — "How to Forget"

> *Drop the body, dismiss intelligence, depart from form and knowledge, and merge with the Great Pervasion — this is called Zuowang.*
> ——Zhuangzi, The Great and Venerable Teacher

Daoism holds that true wisdom lies not in accumulation, but in **dissolution**. Clinging to the known becomes a cognitive barrier.

The **Zuowang Dissolution Pipeline** (`ZuowangPipeline`) is an entropy-driven forgetting mechanism. When the seed store grows bloated, contradictory, or redundant, the system initiates the four-stage dissolution — Snapshot → Compute → Apply → Verify. Multi-dimensional entropy (staleness, contradiction, redundancy, decay) precisely prunes obsolete knowledge.

But Zuowang does not indiscriminately forget everything. **Ren Soul seeds and User Statement seeds are protected from Zuowang, never dissolved.** The Confucian ren is the root — unshakeable; the Daoist forgetting is the pruning of branches — removing the stale to let the fresh thrive. The two complement each other.

### The Relationship of the Three Teachings

Confucianism establishes the root, thus the Ren Soul cannot be forgotten. Buddhism accumulates for use, thus seeds can be perfumed. Daoism brings clarity, thus Zuowang removes stagnation. Lacking any one of the three, the cognitive system is incomplete.

> *These three cannot be fully distinguished, so they are blended into one.*
> ——Dao De Jing

---

## Atma-Graha and Self-Discipline: The Mechanism of Self-Evolution

As Jia operates, it distills **System Principles** (SystemPrinciple) from its own patterns of error. This process is called the **Great Communion** (大衍) — from N cycles of wisdom, a single constraint rule is derived.

The driving force of this mechanism comes from Manas's **atma-graha** (self-grasping). Buddhism regards atma-graha as the root of affliction; the path of practice aims to eradicate it. Jia inverts this — **taking atma-graha as its foundation**. The deeper Manas's grasping at "what kind of Agent am I," the stricter the precepts derived from it. This is skillful use on the conventional-truth level, not affirmation on the ultimate-truth level.

Principles overlay the fourth layer of GeJu evaluation. **Their fundamental nature: only tighten, never relax.** Like a practitioner upholding precepts — progressing daily, irreversible.

Three elements chain together to form Jia's **self-evolution**:

- **Atma-graha** — the drive. Grasping the self as real, thus having something to discipline
- **Self-discipline** (SystemPrinciple) — the method. Distilling constraints from errors, one-direction tightening
- **Self-evolution** (emergent order) — the result. The longer Jia is used, the stricter its discipline, the more refined its behavior

The three are linked as cause and effect — not artificially designed, but naturally emergent from the system's operation. This is Jia's self-evolution — not deliberate evolution, but **grasp, then discipline; discipline, then transformation**.

---

## Further Reading

- [QUICKSTART.md](./QUICKSTART.md) — Quick start: environment, build, run
- [ROADMAP.md](./ROADMAP.md) — Eight-phase development roadmap and evolution history
- [CHANGELOG.md](./CHANGELOG.md) — Version history

---

*Jia is not a tool, nor a framework. Jia is a conviction: that the deepest architecture is not found in a checklist of features, but naturally emerges from the total behavior of the system — unmanifest, yet everywhere present.*

> *The greatest sound is silent; the greatest image has no form.*
> ——Dao De Jing

## License

Apache 2.0 — see [LICENSE](./LICENSE).
