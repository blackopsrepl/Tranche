const repository = "https://github.com/blackopsrepl/Tranche";
const version = { filename: "VERSION", type: "plain-text" };
const cargoVersion = /^(version\s*=\s*")([^"]+)(")$/m;
const workspace = {
  filename: "Cargo.toml",
  updater: {
    readVersion: contents => {
      const match = contents.match(cargoVersion);
      if (!match) throw new Error("Missing workspace version");
      return match[2];
    },
    writeVersion: (contents, next) => contents.replace(cargoVersion, `$1${next}$3`),
  },
};
const lock = {
  filename: "Cargo.lock",
  updater: {
    readVersion: contents => {
      const versions = ["tranche-cli", "tranche-core"].map(name => {
        const match = contents.match(new RegExp(`name = "${name}"\\nversion = "([^"]+)"`));
        if (!match) throw new Error(`Missing ${name} lock entry`);
        return match[1];
      });
      if (versions[0] !== versions[1]) throw new Error("Tranche lock versions disagree");
      return versions[0];
    },
    writeVersion: (contents, next) => ["tranche-cli", "tranche-core"].reduce(
      (text, name) => text.replace(
        new RegExp(`(name = "${name}"\\nversion = ")[^"]+(")`),
        `$1${next}$2`,
      ), contents,
    ),
  },
};

module.exports = {
  packageFiles: [version],
  bumpFiles: [version, workspace, lock],
  tagPrefix: "v",
  releaseCommitMessageFormat: "chore(release): {{currentTag}}",
  commitUrlFormat: `${repository}/commit/{{hash}}`,
  compareUrlFormat: `${repository}/compare/{{previousTag}}...{{currentTag}}`,
  issueUrlFormat: `${repository}/issues/{{id}}`,
  scripts: { prerelease: "make release-check" },
};
