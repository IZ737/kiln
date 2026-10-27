People can sign up to this app, and the home page greets them by name.
Add a settings page where a signed-in person changes their display name:

- `GET /settings` shows a form with the current name in a field named
  `name`, and a hidden field named `csrf` with the session's CSRF token.
  Someone who is not signed in is sent to `/sign-up` instead.
- `POST /settings` with `name` and `csrf` saves the new name (trimmed) and
  redirects to `/settings` with status 303. A name that is empty or longer
  than 80 characters is refused with status 422 and the form again, saying
  what is wrong. A form without the right CSRF token, or from someone not
  signed in, changes nothing and is refused.

Each person may change only their own name.
