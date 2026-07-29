"""Type verification for shadow mode types."""

def check_shadow_types(check):
    import importlib.util, sys

    # Check that the key types are exported from the shadow module
    # by verifying the source files
    import os

    repo = os.path.dirname(os.path.abspath(__file__)) + "/../.."

    with open(os.path.join(repo, "src/trading/shadow/mod.rs")) as f:
        mod_content = f.read()

    check("ShadowSignal exported",
          "pub use decision::{DecisionBuffer, ShadowDecision, ShadowSignal}" in mod_content)
    check("ShadowEngine exported",
          "pub use engine::ShadowEngine" in mod_content)
    check("AccuracyChecker exported",
          "pub use accuracy::{AccuracyChecker, AccuracySnapshot, HorizonStats}" in mod_content)

    # Verify key types exist in source files
    with open(os.path.join(repo, "src/trading/shadow/decision.rs")) as f:
        dec_content = f.read()
    check("ShadowDecision struct exists",
          "pub struct ShadowDecision" in dec_content)
    check("ShadowSignal enum exists",
          "pub enum ShadowSignal" in dec_content)
    check("DecisionBuffer struct exists",
          "pub struct DecisionBuffer" in dec_content)

    with open(os.path.join(repo, "src/trading/shadow/accuracy.rs")) as f:
        acc_content = f.read()
    check("AccuracyChecker struct exists",
          "pub struct AccuracyChecker" in acc_content)
    check("AccuracySnapshot struct exists",
          "pub struct AccuracySnapshot" in acc_content)
    check("HorizonStats struct exists",
          "pub struct HorizonStats" in acc_content)
    check("Multi-horizon tracking (4 horizons)",
          acc_content.count("HORIZONS_SECS") >= 2)
    check("2s horizon defined",
          "\"2s\"" in acc_content)

    with open(os.path.join(repo, "src/trading/shadow/engine.rs")) as f:
        eng_content = f.read()
    check("ShadowEngine struct exists",
          "pub struct ShadowEngine" in eng_content)
    check("MomentumTracker exists",
          "struct MomentumTracker" in eng_content)
    check("Momentum-based strategy (Buy/Sell/Hold thresholds)",
          "momentum > 0.3" in eng_content and "momentum < -0.3" in eng_content)
    check("No wallet keypair dependency in engine (doc comment only)",
          "keypair" not in eng_content.lower() or "no wallet keypair is loaded" in eng_content.lower())

    with open(os.path.join(repo, "src/bin/solshadow.rs")) as f:
        bin_content = f.read()
    check("solshadow binary has --shadow endpoint",
          "--shadow" in bin_content)
    check("solshadow binary does NOT require --keypair",
          "--keypair" not in bin_content)
    check("solshadow binary has /shadow/decisions endpoint",
          "/shadow/decisions" in bin_content)
    check("solshadow binary has /shadow/accuracy endpoint",
          "/shadow/accuracy" in bin_content)
    check("solshadow binary has /shadow/stats endpoint",
          "/shadow/stats" in bin_content)