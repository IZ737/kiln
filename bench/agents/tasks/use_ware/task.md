This app has a contact form written in plain HTML. Rebuild it with the
kiln/ui Ware, which is in the folder `{{KILN_UI}}` (add it as a path
Ware, so it is recorded in kiln.toml and kiln.lock):

- `<ui.Input>` for Name and Email, and `<ui.Textarea>` for Message, each
  with its label;
- `<ui.Button>` to send it;
- the problems a sent form has, shown with `<ui.FormErrors>`, with status 422.

The routes, field names and messages stay as they are.
