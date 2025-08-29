title = "Creating a new Spin release"
template = "spin_main"
date = "2023-07-11T00:22:56Z"
[extra]
url = "https://github.com/spinframework/spin/blob/main/docs/content/release-process.md"

---

# Releasing Spin

This is a guide for releasing a new version of Spin.

The main steps are as follows:

- [Designate a release commit](#designate-release-commit)
- [Switch to release branch and bump versions](#switch-to-release-branch-and-bump-versions)
- [Create the git tag](#create-the-git-tag)
- [Write release notes](#write-release-notes)
- [Notify downstream projects](#notify-downstream-projects)

## Designate release commit

When ready to release the next version of Spin, first locate the commit that will serve as its basis. In other words, the last functional commit to be included besides the version bump commit that will be created below.

Ensure that CI/CD is green for this commit, specifically the [Build](https://github.com/spinframework/spin/actions/workflows/build.yml) workflow and, if applicable, the [Release](https://github.com/spinframework/spin/actions/workflows/release.yml) workflow.

## Switch to release branch and bump versions

1. If this is a major/minor release (e.g. `v2.0.0`) or a first release candidate (e.g. `v2.0.0-rc.1`), create the release branch from the designated commit. The branch name should include the major and minor version but no patch version, e.g. `v2.0`. With our branch protection rules this is easiest from the Github UI with the [New Branch button here](https://github.com/spinframework/spin/branches).

1. Otherwise, if this is a patch release or subsequent release candidate, a release branch will already exist.

   > **Note**: For a patch release, first backport the commits you wish to include to the release branch you're creating the patch release for. Use the [backport script](https://github.com/spinframework/spin/blob/main/.github/gh-backport.sh) to do so, e.g.

   ```
   $ ./.github/gh-backport.sh <pull-request> <branch-name>
   ```

1. Switch to the release branch locally, e.g.

   ```
   $ git checkout <branch-name>
   ```

1. Update versions
   - For example, `2.0.0-pre0` could be `2.0.0` for a major release, `2.0.1` for a patch and `2.0.0-rc.1` for a release candidate
   - Bump the version in Spin's `Cargo.toml`
   - Run `make build update-cargo-locks` so that `Cargo.lock` and example/test `Cargo.lock` files are updated

1. PR these changes to the release branch, ensuring that the pull request has a base corresponding to the release branch (e.g. `v2.0`).

## Create the git tag

> Note: these steps require write permissions to the Spin repo

1. Once the version bump PR is approved and merged, confirm that CI is green for that merge commit.

1. Create a new tag from the merge commit. The tag should begin with a `v`, followed by the version number, e.g. `v2.0.0`. Then, push the tag to the `spinframework/spin` origin repo.

    As an example, via the `git` CLI:

    ```
    # Switch to the release branch
    git checkout v2.0
    git pull

    # Create a GPG-signed and annotated tag
    git tag -s -m "Spin v2.0.0" v2.0.0

    # Push the tag to the remote corresponding to spinframework/spin (here 'origin')
    git push origin v2.0.0
    ```

   This will trigger the [Release](https://github.com/spinframework/spin/actions/workflows/release.yml) workflow which produces and signs release artifacts and uploads them to a GitHub release.

1. If this is a major/minor release, switch back to `main` and update the `Cargo.toml` version again, this time to e.g. `2.1.0-pre0` if `2.1.0` is the next anticipated release.  _(Patch and release candidates can skip this step.)_
   - Run `make build update-cargo-locks` so that `Cargo.lock` and example/test `Cargo.lock` files are updated
   - PR this to `main`

## Write release notes

The [release notes template](./release-notes-template.md) can be used as a guide and starting point.

A good way to familiarize oneself with the features, fixes and other changes in a release is to look at the comparison URL in GitHub,
e.g. `https://github.com/spinframework/spin/compare/<previous tag>...main`. Often commit messages will indicate whether it is a feature, fix,
docs, chore or other PR. However, you may also need to click into the closed pull request linked to a commit to gain more context.

Once the GitHub release is created, edit the release with these notes.

> Note: the GitHub release created by the automation pipeline will come pre-populated with title and changelog. Be sure that the changelog uses the correct previous tag/version. If it does not, edit the release to update the previous tag/version and regenerate the changelog. This auto-generated changelog can be added at the end of the release notes.

## Notify downstream projects

There are a handful of projects that use Spin and would appreciate notification of a new release.

### Spin Docs

- Documentation for Spin exists in the [spinframework/spin-docs](https://github.com/spinframework/spin-docs) repository. Based on the changes remarked upon in the release notes, check to see if any documentation may be missing. If so, either file issues in the repo, create the documentation PR(s) or reach out in the [Spin CNCF Slack channel](https://cloud-native.slack.com/archives/C089NJ9G1V0).

### SpinKube

- The [Containerd Shim Spin](https://github.com/spinframework/containerd-shim-spin) project often organizes its next release around a new version of Spin.

   - Consider announcing the new release in the [SpinKube CNCF Slack channel](https://cloud-native.slack.com/archives/C06PC7JA1EE).
   
   - If a contributor to the project, you might also create a PR bumping Spin crate versions. Often this requires bumping the wasmtime version(s) to suit, as well as orchestrating releases of associated projects with their Spin crate versions bumped to the same, such as:
     - [spin-trigger-command](https://github.com/spinframework/spin-trigger-command)
     - [spin-trigger-sqs](https://github.com/spinframework/spin-trigger-sqs)
     - [spin-trigger-mqtt](https://github.com/spinframework/spin-trigger-mqtt)
     - [spin-trigger-cron](https://github.com/spinframework/spin-trigger-cron)
