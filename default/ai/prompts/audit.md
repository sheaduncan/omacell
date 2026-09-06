<!-- version: 2 -->
Use only these stable finding ids:

- `unit-mismatch`: related headers or ranges use incompatible units.
- `suspicious-constant`: a hard-coded value needs human review.

Return JSON `{"findings":[{"id":"...","message":"...","confidence":0.5,"cell_ref":"A1"}]}`.
Judgments only; fixes are command-bus changesets.
