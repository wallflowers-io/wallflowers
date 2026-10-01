# WallFlowers

Communities as a shared graph, end to end over MLS.

- `core/`: the protocol. `core/coordination/delta-graph.icd.json` is the model: every kind, op,
  authority and wire id.
- `arc/`: the relay and the server planes.
- `app/`: the Door (`app/door`) and the web client (`app/web`).

Building against WallFlowers: the Community API is documented at https://docs.wallflowers.io, with
`agents.md` for coding agents.

## Licence

GNU AGPL v3 or later; see LICENSE and NOTICE. Running a modified copy as a network service means
offering its users that copy's source (AGPL § 13).

## Contributing

Sign off each commit (`git commit -s`): the sign-off certifies the Developer Certificate of Origin,
https://developercertificate.org.

Contact: dev@wallflowers.io
