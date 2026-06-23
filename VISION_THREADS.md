# 🧵 Buzz Threads — context you can hold

> You come back to a channel after lunch. A question turned into a real discussion:
> people answered, an agent investigated, one reply opened a side argument, another
> became the path toward a decision. You do not want the whole conversation dumped on
> you at once. You want to know: _what is this part about, what has been said here,
> and where do I go next?_
>
> Buzz opens the thread around one point of focus. The message is pinned at the top.
> Its answers sit below it. A reply with more underneath shows that depth without
> forcing it into your view. Step in, and that reply becomes the focus. Step back,
> and the larger conversation is still there.

A thread is an attention tool. Its job is to gather enough context around one area of
focus that you can understand it quickly, while keeping the view small enough to
hold in your head.

Buzz needs this more than most chat apps because agents are part of the
conversation. Agents are useful precisely because they can follow tangents, answer
subquestions, and keep working while people are away. They are also very good at
manufacturing depth. If the interface renders every branch they create, the thread
stops being context and becomes terrain.

The answer is not to flatten threads. Flattening throws away real structure: what
answered what, which point a decision came from, where an agent was pulled in. The
answer is also not to render the whole tree. A full tree may be complete, but it
asks a person to keep too much in view at once.

Buzz treats a thread as a focused window: one message, its answers, and a clear
way deeper. The tree remains in storage. The experience stays within a human span
of attention.

Nothing here is novel on its own. Threaded storage is solved. Drill-down views are
solved. Reply composers and agent context loading are solved. The insight is that
they should all share the same boundary: the part of the conversation a person is
trying to hold right now. Each piece is small. The shared focus is the thing.

---

## What You See

You open a thread in the side pane. There is always a focused message pinned at
the top: the thing this view is about right now.

Below it are the direct replies. You can read the local conversation without
decoding indentation, connectors, or `@mention` scaffolding. Every reply is
answering the thing at the top.

If a reply has more underneath, Buzz shows that there is more to enter. You can
open one level in place to preview the next layer. If you want to follow that
branch further, you step into it and it becomes the new focus. The pane does not
keep squeezing rightward. It gives you a fresh, readable view of the same shape:
a message, its answers, and a way in.

```
focused root
├─ reply               <- children: always shown
│  └─ Show 4 replies   <- grandchildren: one tap, in place
└─ reply
   └─ Show 7 replies   <- one level deeper re-roots onto this reply
```

A labeled way back keeps depth from feeling like getting lost. You are always
somewhere specific, and you can always return to the context that brought you
there.

The result is not a clever tree renderer. It is a way to arrive late, gather the
local context, and decide whether this is the branch you need to follow.

---

## Why It Matters

Most of the time, opening a thread is not a desire to explore a complete data
structure. It is a desire to answer a practical question: _what is happening
here?_

The focused window answers that question quickly. It gives you the message that
set the local context, the replies that matter most immediately, and visible
signals for where the conversation continues. You can gather enough context to
participate without paying the cost of the whole tree.

That boundedness is the point. A side pane is narrow. A phone screen is narrower.
An agent's useful context budget is finite too. The same principle should hold
across all of them: load the part of the conversation that is relevant to the
current focus, not every branch that happens to descend from the same root.

This is also a performance promise. A deep thread should not lock the app just
because the storage tree is large. Rendering a bounded slice means there is less
markdown to parse, less UI to commit, and less work on the main thread. The
attention model and the rendering model point in the same direction: show less,
but make the less be the right part.

---

## Replies Stay Shallow

Depth is something you opt into, not something that happens to you.

For people, the composer targets the level they are actually reading. Open a
thread and you are replying to its focus. Step into a reply and you are replying
there. Reading down a conversation should usually add a sibling at the level you
are looking at, not a new leaf buried three levels deeper than you intended.

For agents, the same discipline matters even more. When an agent answers a point,
the default should prefer the local parent over the deepest descendant. Agents are
the main source of runaway depth; keeping their replies shallow at the source is
more durable than only hiding the depth after they create it.

The goal is not to prevent deep conversations. Some work really does need a chain
of follow-up questions. The goal is to make depth a deliberate path, not the
default shape of every response.

---

## One Context

The focused window is not just a rendering choice. It is the definition of the
conversation the human is trying to share with an agent.

Today, an agent pulled into a thread can grab the root plus a flat scoop of
recent replies from anywhere underneath it. That ignores where the human actually
tapped. The agent may be looking at the same thread in storage while missing the
same area of focus in experience.

Load the focused window instead: the focused message, the path back to the root,
and the replies immediately around that focus. Then the human and the agent are
looking at the same conversation. The agent is not summoned into an abstract tree.
It is summoned into the part of the discussion the person is holding right now.

One definition of "the thread," for both.

That is the experience Buzz wants everywhere humans and agents collaborate: not
"the agent read a lot of nearby messages," but "the agent joined me here."

---

## How It Works

The tree already exists. Every reply stores its parent and its root with NIP-10
`e` tags, capped at depth 100 at ingest. Nothing about storage needs to change.

The change is in what Buzz chooses to visualize, what the composer chooses to
target, and what context agents receive. The storage model can remain complete
while the reading model stays bounded.

This also means there is no separate rich desktop tree to design. Threads live in
a side pane, and a side pane is already closer to a phone than to a full desktop
canvas. The same focused-window model works on desktop and mobile, with
platform-appropriate chrome.

---

## Honest Costs

Capping visible depth gives up the bird's-eye view. Once you step into one
branch, you cannot also see everything a sibling branch concluded. That is a real
tradeoff.

Buzz is betting comprehension beats completeness. The rare cross-branch reference
is better served by a quote, a link, or a summary than by making every thread
render its full depth all the time.

There are also things this does not try to solve yet. Buzz can still add ranking
later: one "top chain," engagement sorting, or other ways to surface the most
important path through a busy thread. A hard token budget for agent context is
also still needed. The focused window does not replace that budget; it gives the
budget a meaningful shape.

---

## The Point

A thread is a tree in storage. It should be a focused conversation in experience.

Show the part that matters now. Keep it small enough to hold. Let people and
agents step deeper only when deeper is where the work is.

---

_Buzz 🐝 — context you can hold._
