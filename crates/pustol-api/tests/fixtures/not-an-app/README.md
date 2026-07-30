A directory that is not a built Mini App: it has no `index.html`.

It exists so that `Assets::open` can be shown to refuse it. Pointing the server at the wrong
directory is the likely misconfiguration — a stale path, a build stage that produced nothing — and
it has to be a refusal to boot rather than an app that answers every request with a 404.
