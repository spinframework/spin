## Spin <version>

The <version> release of Spin brings a number of features, improvements and bug fixes.

Some highlights in <version> at a glance:
- <new feature linked to PR>
- <new feature linked to PR>

<List notable fixes, deprecations, breaking changes, etc.>

As always, thanks to contributors old and new for helping improve Spin on a daily basis! 🎉

### Verifying the Release Signature

After downloading the <version> release of Spin, either via the artifact attached to this release corresponding to your OS/architecture combination or via the [installation method of your choice](https://spinframework.dev/install), you are ready to verify the release signature.

First, install [cosign](https://docs.sigstore.dev/cosign/system_config/installation/). This is the tool we'll use to perform signature verification. Then run the following command:

```
cosign verify-blob \
    --signature spin.sig --certificate crt.pem \
    --certificate-identity https://github.com/spinframework/spin/.github/workflows/release.yml@refs/tags/<version> \
    --certificate-oidc-issuer https://token.actions.githubusercontent.com \
    --certificate-github-workflow-sha <commit_sha> \
    --certificate-github-workflow-repository spinframework/spin \
    spin
```

If the verification passed, you should see:
```
Verified OK
```

## Full Changelog
<Copy/paste the auto-generated release changelog that GitHub produces here>