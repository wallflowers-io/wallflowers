# nova — User Guide (draft)

*The private, peer-to-peer collaboration tool where the work and people you keep private quietly surface the connections you would have missed. This is the guide for the first version. Where a section is still being decided, it says so on purpose: a manual you cannot write cleanly is a design that is not finished.*

## What nova is

nova is a **private, peer-to-peer project management and collaboration tool**. You run projects, hold conversations, and do real work with people you trust. That part looks familiar.

What makes it nova: an on-device agent reads *your own* context and surfaces connections across projects that nobody made public. The sensor you are quietly building turns out to fit someone else's satellite, and nova is the only place that could have known, because the sensor was never on a profile. Your context is read and searched **on your device**, and nova never sees it. The only thing that crosses the network is the connection worth making.

## Getting in

You do not sign up. **You get invited to a project.** You open the invite and you are immediately in a conversation about that project, with the people in it. No empty state, no "set up your profile," no graph to build first. You are collaborating from the first screen. Everything else, your context and the discovery, fills in behind it.

Your identity is a key created on your device the first time you join. There is no account and no central directory of users.

## Connecting your context

Discovery is only as good as what nova can read on your behalf. You connect your sources, mail, docs, code, notes, through **Nango**, a standard integration layer. Nango delivers your data to your device, where nova builds a private index of it.

Connecting a source means authorizing a pipe, the same kind of connection you already grant Gmail, Linear, or any other tool you use. That is the honest boundary: Nango, a connector you authorize, transits your data the way every integration does. What you are *not* doing is handing your collaboration to a central platform that keeps it. nova the network, and the people on it, never see your data or the index built from it.

You choose what to connect. Disconnect a source any time and its on-device index is wiped.

## Projects and objectives

A project has people, a conversation, and an **objective**: the thing you are trying to do together. The objective is what discovery runs against.

*Still being decided: how structured an objective is (free text vs guided). Not on the critical path for the first version.*

## How nova finds things for you

This is the magic, and it is an agent, not a search box. nova runs a **CLA**, an agent with tools, over your private on-device index against a project's objective, hunting for non-obvious fits: the component of yours that belongs in someone else's project. It reasons; it does not keyword-match. The quality of this is the product, and it is where the real work goes, prompt design and the tools the agent can use.

You never see anyone else's data and they never see yours. nova carries only the *objective* out to each member's device, and the *surfaced fit* back to the project. The graphs stay home.

## When nova finds something

You get a **notification**: "your sensor might fit Team B's satellite." That notification is the entire point of nova. It is the moment the app earns its place on your phone, and it is what makes nova something people pay for: an agent that finds you the introductions and the components you did not know to look for.

From the notification you act: open the fit, request the intro, pull the component into your project, or dismiss it.

## What nova grows into

The first version is for working groups, your batch, your collaborators, the people you are building with now. The same shape extends: systems engineering across teams, and eventually live production systems as shared, monitored objects. The collaboration space is the substrate; the reach grows from a group of founders to the systems they run. *(Horizon, not the MVP.)*

## What nova never sees

**nova never sees your graph.** The private index of everything you connected lives on your device, and discovery runs there. The network, and the people on it, see only what you choose to surface: an objective you posted, a fit you shared. Not your mail, not your docs, not your code.

There is no central platform holding your projects, your messages, or your social graph. Your collaboration is peer-to-peer and end-to-end encrypted between you and the people in it. Connecting a source routes through Nango as a pipe to get your data onto your device, an integration you authorize, not a platform you hand your work to.

So the privacy claim here is precise, and it is the kind that survives a technical user reading their own network traffic: **no one owns your network but you, and nova never sees your context.** That is the reason the agent can read everything, because everything it learns stays with you.
