use shadow_rs::BuildPattern;
use shadow_rs::ShadowBuilder;

fn main() -> shadow_rs::SdResult<()> {
    ShadowBuilder::builder()
        .build_pattern(BuildPattern::Lazy)
        .build()
        .map(|_| ())
}
