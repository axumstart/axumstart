# Website

This website is built using [Docusaurus](https://docusaurus.io/), a modern static website generator.

## Installation

```bash
npm install
```

**Note**: feel free to use the package manager of your choice.

## Local Development

```bash
npm run start
```

This command starts a local development server and opens up a browser window. Most changes are reflected live without having to restart the server.

## Build

```bash
npm run build
```

This command generates static content into the `build` directory and can be served using any static contents hosting service.

## Deployment

The repository workflow builds the site and synchronizes `docs/build/` to the configured deployment
host over SSH. It requires `SSH_PRIVATE_KEY`, `SSH_KNOWN_HOSTS`, `SSH_PORT`, `SSH_USER`,
`SSH_HOST`, and `SSH_TARGET_PATH` repository secrets. `SSH_KNOWN_HOSTS` is a reviewed host-key
entry; the workflow does not trust a live `ssh-keyscan` result.

Deployment is performed by `.github/workflows/deploy-docs.yml`; local `npm run build` is the
supported way to verify the generated site.
