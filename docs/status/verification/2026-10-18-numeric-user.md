# 2026-10-18 — The dashboard and checkout images declare a numeric `USER`

Branch `fix/frontends-numeric-user-run-as-non-root`, from `master` at
`c8c1904`. One Dockerfile change, comment and status corrections. **No chart
template behaviour, no `values.yaml` default, no chart version, no migration,
no route and no `NotImplemented` token moved.**

## What was wrong

Enabling `dashboard.enabled: true` from chart 0.6.0 on the hetzner-prod
cluster (2026-10-18) rendered a pod the kubelet refused to start:

```text
CreateContainerConfigError: container has runAsNonRoot and image has
non-numeric user (node), cannot verify user is non-root
```

`deployment-dashboard.yaml` and `deployment-checkout.yaml` set
`runAsNonRoot: true` and, on purpose, no `runAsUser` (the image is meant to be
the single source of truth for the UID). The kubelet can verify non-root only
from a **numeric** image `USER`, or from a pod `runAsUser`. Both
`frontends/Dockerfile` stages, `runner` (the dashboard) and `checkout`, ended
in `USER node`.

The repository's own evidence for "runs non-root" came from plain
`docker run` (`--read-only --tmpfs /tmp`, and `--user 1000:1000` for the
dashboard), which does not perform the kubelet's check. So that evidence was
true of what it measured and never proved what the chart's comments and
status rows used it for. The `checkout` template has the same latent bug; it
was not hit only because `checkout.enabled` has not been turned on in a
cluster.

On the published `ghcr.io/vaam-apps/vpay-dashboard:0.6.0`, in a throwaway pod:
`uid=1000(node) gid=1000(node)`, and `/etc/passwd` has
`node:x:1000:1000::/home/node:/bin/sh`.

## The change

`frontends/Dockerfile`: both `USER node` lines become `USER 1000:1000`, with a
comment saying it is the `node` user of `node:22-alpine` and must be numeric
for the kubelet's `runAsNonRoot` check. The chart templates are unchanged in
behaviour; their comments, `values.yaml`, `deploy/helm/vpay/README.md`,
`_validate.tpl`'s retired-guard note, `docs/flows/deployment.md`,
ADR-0022's blocking-condition paragraph and three rows of
[../infrastructure.md](../infrastructure.md) said the image's _name_ sufficed
("the image has an `/etc/passwd`", "no invented UID"); they now say it does
not, in the page's own dated-correction style.

**This does not change any published image.** Images up to and including
`0.6.0` still carry the named user. The fix reaches a cluster with the next
release that rebuilds `vpay-dashboard` and `vpay-checkout`.

## What ran, on 2026-10-18 (macOS arm64, Docker 29.8.0)

Both targets built from the repository root:

```text
$ docker build -f frontends/Dockerfile --target runner   -t vpay-dashboard:numeric-user .   # exit 0
$ docker build -f frontends/Dockerfile --target checkout -t vpay-checkout:numeric-user  .   # exit 0
$ docker inspect --format '{{.Config.User}}' vpay-dashboard:numeric-user
1000:1000
$ docker inspect --format '{{.Config.User}}' vpay-checkout:numeric-user
1000:1000
```

Each started as that user with no `--user` flag, under a read-only root and a
`/tmp` tmpfs, and answered its health route:

```text
$ docker run -d --read-only --tmpfs /tmp -p 13000:3000 vpay-dashboard:numeric-user
$ curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:13000/healthz
200
$ docker exec <container> ps -o user,pid,args
USER     PID   COMMAND
node         1 next-server (v15.5.27)
$ docker run --rm --entrypoint sh vpay-dashboard:numeric-user -c 'id'
uid=1000(node) gid=1000(node) groups=1000(node)
```

and the same `200` / `node` / `next-server` for `vpay-checkout:numeric-user`
on port 13001.

`just helm-check` (lint of four value sets, four renders, 24 guards,
kubeconform): `helm-check: ok`, `68 resources found in 4 files - Valid: 68`.

## What did not run

- **No pod.** No cluster was touched. The kubelet's `runAsNonRoot` check
  itself was not exercised against the new image; the evidence is that
  `Config.User` is the numeric `1000:1000`, which is the condition the
  kubelet's error message names. The first real confirmation is the next
  deployment of a release that includes this change.
- No Dockerfile linter: `hadolint` is not installed on this machine and no
  gate in `just ci` runs one.
