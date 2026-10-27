# kiln/ui

Accessible components for server-rendered Kiln pages and forms: ordinary
Glazes with scoped CSS, styled by your design tokens.

| Component | |
| --- | --- |
| `<ui.Button type="submit" tone={ui.Tone.Primary}>Save</ui.Button>` | A button; `tone` is `Neutral`, `Primary`, `Danger` or `Success` |
| `<ui.Input label="Email" name="email" type="email" error={...} />` | A labelled field, with `hint` and `error` tied to it |
| `<ui.Textarea label="Body" name="body" />` | A labelled text area |
| `<ui.Select label="Role" name="role" options={[ui.choice("a", "Admin")]} />` | A labelled choice |
| `<ui.Checkbox label="Published" name="published" checked={...} />` | A checkbox and its label |
| `<ui.Radio label="Plan" name="plan" options={[ui.choice("free", "Free")]} selected="free" />` | Radio buttons under one legend |
| `<ui.FormErrors problems={checked.messages()} />` | A form's problems, announced to screen readers |
| `<ui.Alert tone={ui.Tone.Success} title="Saved">...</ui.Alert>` | A message; danger alerts are announced at once |
| `<ui.Badge>Draft</ui.Badge>` | A short label |
| `<ui.Card title="Posts">...</ui.Card>` | A box of content, with an `actions` slot |
| `<ui.EmptyState title="No posts yet">...</ui.EmptyState>` | An empty list |

The colours, spacing and radii come from CSS custom properties with
defaults: `--color-accent`, `--color-on-accent`, `--color-surface`,
`--color-text`, `--color-muted`, `--color-border`, `--color-danger`,
`--color-success`, `--color-focus`, `--space-sm`, `--space-md`,
`--radius-card`. A Phase 3 theme that sets them restyles every component.

To own a component, copy it into your project: `kiln ui add button`
writes `components/button.glaze` (and `components/ui.kiln`), which you
then use as `<components.Button>` and change freely.

Automated tests check labels, roles and keyboard focus; they cannot prove
a page is accessible. Review your pages with a screen reader too.
