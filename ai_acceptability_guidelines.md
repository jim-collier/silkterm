<!-- markdownlint-disable MD007 -- Unordered list indentation -->
<!-- markdownlint-disable MD010 -- No hard tabs -->
<!-- markdownlint-disable MD033 -- No inline html -->
<!-- markdownlint-disable MD055 -- Table pipe style [Expected: leading_and_trailing; Actual: leading_only; Missing trailing pipe] -->
<!-- markdownlint-disable MD041 -- First line in a file should be a top-level heading -->

<!-- TOC ignore:true -->
# AI acceptability guidelines

Where AI is allowed near this project, where it isn't, and who is accountable either way.

<!-- TOC ignore:true -->
## Table of contents

<!-- TOC -->

- [Introduction](#introduction)
- [Problems with AI](#problems-with-ai)
	- [Environmental](#environmental)
	- [Economic](#economic)
	- [Ethical](#ethical)
	- [Code quality](#code-quality)
	- [Security risks](#security-risks)
	- [Bad PR](#bad-pr)
- [Non-problems with AI](#non-problems-with-ai)
- [Good uses of AI](#good-uses-of-ai)
	- [Review and analysis](#review-and-analysis)
	- [Tests and tooling](#tests-and-tooling)
	- [Hard problems](#hard-problems)
	- [Porting to other languages](#porting-to-other-languages)
	- [Porting to other operating systems](#porting-to-other-operating-systems)
	- [Tedious non-coding tasks that pay nothing](#tedious-non-coding-tasks-that-pay-nothing)
- [Rules for AI use in this project](#rules-for-ai-use-in-this-project)
	- [A person is accountable for every merged line](#a-person-is-accountable-for-every-merged-line)
	- [Self-assessment of AI speedup is not evidence](#self-assessment-of-ai-speedup-is-not-evidence)
	- [What AI may do with light review](#what-ai-may-do-with-light-review)
	- [What always needs full human review](#what-always-needs-full-human-review)
	- [What AI does not decide](#what-ai-does-not-decide)
	- [Contributing](#contributing)
- [Where this could change](#where-this-could-change)
- [The use of AI in writing this document](#the-use-of-ai-in-writing-this-document)

<!-- /TOC -->

## Introduction

This project's original author has decades of programming experience. (Hobby, then professional, then hobby.) With a whole career involving software development.

When AI started becoming a "thing" in programming, I (the original author) was against it. Which was convenient, because it sucked at it.

But that view on AI in programming has been softening, with the emergence of frontier models like Claude Fable.

I've been running "AI-led" experiments under an alt GitHub account, mostly to find where these tools break, and where they can be useful. Many documented hard lessons-learned are coming out of that work.

One result is that this nearly decade-old main account (as of 2026) no longer has a blanket "no-AI" policy.

This document is a first pass at specifically documenting what, when, and how AI is allowed into this and other projects.

## Problems with AI

Feel free to skip this section. It may not be stating anything you don't already know. It's mostly just stating "for the record", an awareness of some of the main problems.

### Environmental

This is the hardest one to justify, and I have no good answer to it. Using these tools, while believing the current cost is indefensible, is ...well, hypocrisy.

The scale is not in dispute. US data centers used about 4.4% of the country's electricity in 2023, and [Berkeley Lab](https://newscenter.lbl.gov/2025/01/15/berkeley-lab-report-evaluates-increase-in-electricity-demand-from-data-centers/) puts 2028 somewhere between 6.7% and 12%.

The argument is not that this is fine. It is that cost per unit of useful work is falling much faster than the headlines suggest:

- Querying a model at GPT-3.5 quality fell from $20 to $0.07 per million tokens between late 2022 and late 2024, per the [Stanford AI Index](https://hai.stanford.edu/ai-index/2025-ai-index-report). Roughly 280x in under two years.

- Hardware cost per unit of performance is dropping about 30% a year. Energy efficiency is improving about 40% a year.

- Open-weight models have nearly caught up. The benchmark gap against closed models narrowed from 8% to 1.7% in a single year.

The part that matters here: open-weight coding models that fit in 24 to 32 GB on a consumer GPU now score around 80% on SWE-bench Verified, against roughly 90 to 95% for the best hosted models. That is already enough for the review and testing work described below.

On capability, the number usually quoted is [METR's](https://metr.org/blog/2025-03-19-measuring-ai-ability-to-complete-long-tasks/): the length of task a model finishes with 50% reliability has doubled about every seven months over the six years to 2025. Fitting only the 2024-2025 data gives a steeper curve. METR puts no single number on that, but on SWE-bench Verified alone they measured a doubling time under three months.

While not a perfect metric of "coding power", the extrapolation is inescapable: at seven months, 3.3 years is about 50x; at four months it is nearer 1000x. A pretty clear direction, not necessarily a prediction.

So the position is narrower than "AI is worth it". It is that the useful capability is on track to run on a desktop with no cloud data center behind it, and sooner is better.

**A solar-powered desktop running a Claude Fable-level model good enough for adversarial code review is not a fantasy. It's most of the way here**.

The only question after that transformational milestone is: Can we be individually satisfied with that level of quality code contribution - or exponentially ever more hungry for "more"?

If you can write your own personal feature-complete Adobe Creative Suite for Linux just for yourself, and never have to rent another product from Adobe ever again - would you? At just the cost of one more dried-up farm well? (OTOH, the same "wait until that power hits your solar-powered desktop" argument could apply there too. Some will always want it "now" at any externalized cost to anyone/everyone; some will always be able to wait, and that formula might never change.)

### Economic

The companies leading this are among the largest ever to exist, and may be steering the global economy toward a cliff. If that happens the human cost could be severe, and will fall mostly on people who never opted in.

Time will tell whether this is the largest bubble in history or whether the anticipated returns arrive first. There are non-imaginary scenarios where that could happen, outside of inside investors intending to find bag holders for their well-timed exits.

If guidelines like those outlined below were adopted broadly - narrow scope, human accountability, no AI making the decisions that matter - and especially limited to near-future desktop models - then global demand might look less like a gold rush. (I mean sure that may be cope. But it may also be true.)

### Ethical

These are mostly downstream of the previous two.

The clearest measured harm so far is to entry-level work. Stanford's [Canaries in the Coal Mine](https://siepr.stanford.edu/publications/working-paper/canaries-coal-mine-six-facts-about-recent-employment-effects-artificial) found a 13% relative decline in employment for workers aged 22 to 25 in the most AI-exposed occupations, including software development, while employment for more experienced workers in those same occupations held steady.

That's not AI's "fault", it's decisions made by employers with AI used as a rationale - possibly against their own long-term interests. A profession that stops training juniors runs out of seniors.

This project can't fix this problem, and might be presumptuous to even discuss it. But what it/I can do is at least not pretend the problem is imaginary.

### Code quality

Code quality was a serious and universal problem until recently, even on small projects. But even on the latest frontier models, it needs to be carefully managed.

One main risk is not that the code fails to work. It is that it works and quietly rots. [GitClear's analysis](https://www.gitclear.com/ai_assistant_code_quality_2025_research) of 211 million changed lines found 2024 was the first year on record where copy-pasted code exceeded moved code, with code clones up roughly fourfold. Refactoring went from about a quarter of changed lines in 2021 to under a tenth in 2024.

That is the failure mode to watch: a model asked for a fix writes a new version rather than finding the existing one. Nothing breaks. The codebase just gets worse in a way no test catches.

### Security risks

Running an agentic tool on a development machine is a risk to the developer. It reads local files, runs commands, and reaches the network. Know the risks and act accordingly.

The generated code is another risk. Veracode's [2025 report](https://www.veracode.com/resources/analyst-reports/2025-genai-code-security-report/) found that across 80 tasks and 100+ models, 45% of samples introduced an OWASP Top 10 vulnerability. Java was worst at over 70%. Cross-site scripting was missed in 86% of the cases where it applied.

Hallucinated dependencies are a nastier problem. A [study of 576,000 generated samples](https://www.usenix.org/system/files/conference/usenixsecurity25/sec25cycle1-prepub-742-spracklen.pdf) found package names that do not exist in about 5% of commercial-model output and about 22% of open-model output. Attackers register the common ones and wait.

Every dependency an AI suggests absolutely must be human-verified.

But used the other way around - for adversarial review, security review, and fuzz and regression suites - the same tools measurably improve a project.

**That asymmetry in security is a driving factor behind these guidelines**.

### Bad PR

Any project accepting AI-generated contributions carries this risk, including this one.

AI is becoming a public enemy. The cause is (probably) mostly greed-driven hype, the economics, and the short-sighted shady tactics used to foist data centers onto communities whose citizens pay the externalities.

That perception will probably/hopefully shift once desktop-class open models are good enough to work offline, which looks like a short wait rather than a long one. (And for the love of Gorn don't confuse that concept with "Copilot"!)

Either way, hiding the involvement of AI is not the way out of AI's growing PR problem. I believe that being transparent about its use is the "right" and principled path forward, along with managing it effectively as a tool - with an attempt at objective reasoning, effectiveness measurement, and global facts at hand. And then, accepting whatever criticism may come as a result - whether grounded in fact and reality or irrational fear - and well-deserved or not. (And the lines between them may not always be objectively obvious.)

## Non-problems with AI

The common objection "LLMs don't understand context" is fundamentally wrong at worst, or confused at best.

And either way, is specifically regarded as *not* a problem (or really an "anti-problem") for this project, and github account.

Contextualization isn't something bolted onto an LLM; it *is* the mechanism.

They are literally *context engines*, by design.

But the human objection usually means something narrower: *situational* context. In other words, who you are, what you ate this morning (maybe too many carbs), and what's really at stake here. So that would be a fair criticism, just poorly worded.

Where LLMs vastly exceed us is textual context: conditioning on a hundred thousand things at once, across domains no single person could ever read let alone remember.

And "understand" belongs in quotes, because nobody can specify what "understanding" *is*, beyond what it *does*. (Try it.)

We grant "understanding" to other humans on inference alone, for free: they presumably have the same substrate, the same evolutionary origins, and we've each got one running locally.

But in the end, we can't be sure *any* intelligence - yours, mine, the pilot of your next flight - is anything more than next-word-prediction machines, running on wetware, that got good enough at not dying to mistake itself for something else.

An "optimal" AI or AGI may or may not eventually be LLM-based. There is ongoing R&D that may yield superior "intelligence" that looks utterly alien to us - maybe that we can't even recognize. Also, "intelligence" rooted in human language, history, and perspective (and our monkey fears and biases and scifi novels) - may be too limiting if not outright dangerous. But the advantage of LLMs at least for now, is that 1) they *can* communicate with us, and 2) we can literally watch them think, in our own native language.

Another advantage of LLMs specifically for coding - and more to the point here - is that the inventors of computer languages borrowed the machinery of linguistics - which LLMs happen to be particularly well-suited for dealing with.

## Good uses of AI

What current LLMs demonstrably do well:

- Hold *vastly* more context at once than a person can. A model reads an *entire repository* in one pass. Human working memory holds roughly *four things*.

- Combine ideas across fields that no single reviewer has read, or even can in a lifetime.

- Produce solutions that do not appear in their training data. (Contrary to popular belief.)

- Look up information they were never trained on.

What they cannot do:

- Retain anything between sessions (without memory files),

- Exist as one continuous mind.

Our rules for AI contribution are built, in part, with these limits in mind.

As mentioned before, code also happens to suit LLMs well. Parsing, tokens, syntax, semantics, grammar - the vocabulary of programming was literally created from linguistics in the first place. That is a large part of why these models got good at code before they got good at most other technical work.

### Review and analysis

Verification is where LLMs are strongest, because a wrong answer is cheaper than a defect.

- Adversarial code reviews are arguably the most valuable use of AI in coding. The ginormous context windows of LLMs are able to trace through large and multiple branches of code paths and hold more in mind at once, than any human. Hallucination is not (or does not seem to be) an issue. And LLMs don't get bored to tears.

- Security reviews: input handling, allowlists, deserialization, anything touching a file path or a URL scheme.

- Performance review. Reading hot paths for repeated work, redundant allocation, and locks held longer than they need to be.

- Triaging linter, static analyzer, and profiler output. These produce more findings than anyone can possibly read. Sorting the real ones from the noise, with reasons attached, is a perfect use for AI.

A model that reports a defect can usually write the fix too. Those patches are accepted on the same terms as any other: read, understand, cover with a test.

### Tests and tooling

- Regression suites, especially the pinning kind written after a bug is understood, to prove it stays fixed.

- Fuzz harnesses. Tedious to write, valuable to have, and easy to check.

- CI/CD and build pipelines. Usually pretty easy to create - but just so tedious and error-prone. Immediate feedback, verifiable by running.

### Hard problems

- Bugs needing more context than one person can hold at once, where the cause is spread across several files and a dozen interacting conditions. This is a genuine human/AI delta, not just a speed increase.

- Math, algorithms, and logic that have published academic literature behind them. First of all, the LLMs are aware of the papers in the first place, even obscure historical ones. Secondly, it has already read them. Third, it "*understands*" them, in the context of how it might apply to your particular problem. It's not necessarily "better than human" - just profoundly faster at doing the research and applying - or rejecting - the findings.

### Porting to other languages

AI truly shines at taking a solid, well-defined and documented codebase, with comprehensive existing tests harnesses - and porting it to other (or additional) languages. The latest frontier models can do so with high fidelity, and while optionally (ideally) converting to target language native idioms.

It is then a fully human responsibility to:

- Read and understand the generated code.

- Insure the code conforms to house style. (Which should be at the linting and autoformatting stage but still needs eyeballs.)

- Make sure the it passes all automated *and* manual unit, integration, usability, UAT, regression, performance, and security tests.

### Porting to other operating systems

This is similar to the previous point. Depending on the language (e.g. Go/Rust/Zig and non-compiled cross-platform scripting languages), this may be a trivial non-AI task that should just be part of the CI pipeline.

But depending on what the program is doing, there is often OS-specific branching the code has to take, for functionality that isn't covered by the language's own cross-platform standard library. AI is often better than humans at just "knowing" how to optimally and idiomatically handle such cases.

But the human responsibility picks back up at the end, just as with the "porting to other languages" section.

### Tedious non-coding tasks that pay nothing

Examples:

- Demo gif and video generation. AI is quite good at this (including fully anonymized synthetic scenarios), where for humans it is incredibly tedious and not fun for anyone - generally neither technical nor creative types.

- Benchmarking and measurements for competitive comparison charts.

- Asset generation. This is a tougher call, as creatives need work too and are being replaced by AI at heartbreaking levels. But for assets on a FLOSS project with no pay, and no one stepping up to volunteer their creative blood sweat and tears, what are you going to do? Personally, I'm artistic enough - and experienced enough with the tools - to generate image, audio, and video assets by hand. But it is extremely time-consuming, and I'd rather be putting that time to where it matters most: product design, problem-solving, and coding. For the assets, I usually know exactly what I want, and can describe it precisely. That's a good use of AI (at least in those isolated terms).

- Boring "required" website setup and generation. I don't mean websites that are the core function of a product and where programmers, product designers, human factors designers, artists, graybeard seniors, back-end engineers, stakeholders and investors come together to make something wonderful. I mean the boring "required" bare-minimum web presence that even basic FLOSS products need, but no one wants to take on the torturous tedium of putting together without getting paid for doing. Such websites are (arguably) commodities - not big-brain creative work.

## Rules for AI use in this project

### A person is accountable for every merged line

AI is not an author and not a defense. Whoever merges a change owns it, answers questions about it, and fixes it when it breaks. "The model wrote it" is not a thing anyone gets to say as a defense.

The practical test: anyone who cannot explain a change does not merge it.

### Self-assessment of AI speedup is not evidence

METR ran a [randomized trial](https://metr.org/blog/2025-07-10-early-2025-ai-experienced-os-dev-study/) with experienced open-source developers on repositories they already knew well. They were 19% slower with AI tools. They believed they had been 20% faster.

That gap is the important part. Perceived productivity is not measurable by the person experiencing it, so "it's faster this way" carries no weight on its own. Where speed matters, measure it.

That trial ran on early-2025 tools, and METR now flags it as out of date. Their [February 2026 follow-up](https://metr.org/blog/2026-02-24-uplift-update/) on late-2025 tools estimates a speedup instead: about 18% for returning participants and 4% for new ones, though the confidence intervals straddle zero in both cases and the authors warn of heavy selection bias, since developers increasingly refused to participate without AI.

So the direction has moved. The lesson about self-report has not.

### What AI may do with light review

- Read and summarize existing code.

- Review a diff and report findings.

- Draft tests, build scripts, and CI configuration.

- Sort and explain linter, analyzer, and profiler output.

### What always needs full human review

Everything that reaches the repository. Specifically:

- Any change to the security boundary. Scheme allowlists, path handling, deserialization, anything spawning a process.

- Any new dependency. Confirm it exists, is the package intended, and is actually maintained.

- Any generated test. A test that asserts current behavior locks in current bugs, and reads as passing coverage while proving nothing.

- Any change described as a refactor. This is where duplication gets introduced.

### What AI does not decide

- Architecture, and anything that will be expensive to reverse. AI can suggest things - even be asked for options. But not decide.

- What gets released and what gets held back.

- Anything requiring judgment about users rather than about code.

Explaining a tradeoff is useful. Choosing it is not delegated.

### Contributing

Contributions that used AI will not be automatically rejected. Two conditions:

- Say so in the pull request, along with roughly what it was used for. No process detail needed, no apology expected.

- Submit it as work that is understood, tested, and stood behind.

Commit messages describe the change, not the tooling used to make it - the same convention that applies to editors, formatters, and everything else.

## Where this could change

The position stated here is conditional. Things that would tighten it:

- Evidence that these review uses cost more in defects than they catch.

- Contribution volume that makes the review load unsustainable.

Things that would relax it:

- Desktop-class open models that close the remaining gap on agentic work.

- A power story that does not require a hyperscale data center.

The second list is closer than it looks. That is most of why this document exists rather than a shorter one saying no.

## The use of AI in writing this document

Everyone who knows me, knows I love to write. *A lot*. Especially a lot about highly technical subjects.

But I used AI to (try to) help improve this document, specifically:

- **Spell-checking**. I normally use LibreOffice Writer for spell-checking, but it tediously complains about every part of every URL and Markdown link. My exceptions library is ginormous now. AI knows what to skip.

- **Fact-checking**: AI cautioned me to remove or back off several claims that were overstated at best, or flat-out wrong at worst. This may be the second-best enhancement AI has brought to my writing. (And sometimes I learn new things - independent of research - along the way.)

- **Reducing "Conclusion Shopping" and Confirmation Bias**. We all conclusion-shop for studies and links to support our preconceived arguments. I'm no different. AI is a great tool to help keep this bad habit in check. Possibly the best single AI-related improvement a solo writer can do to help keep writing honest.

What I did *not* use AI for in this document:

- **Content generation**. Every questionable and/or redundant argument made here, every odd injection of unsolicited opinion and narration into what should be a straightforward "guidelines" document, is from a human. This human.

- **Grammar-checking**. While I probably should have used AI for grammar-checking - and it's arguably better than privacy-invading tools like Grammarly - I prefer the organic feel of my own tedious phrasing, run-on sentences - and abruptly ending such run-on sentences where I've run out of examples but want it to *seem* like there's more, with ", etc.". If it's not tedious for me to read my own writing, it just doesn't *feel* right.

- **Tone and appropriateness policing**. Again: probably would have been a good idea.
