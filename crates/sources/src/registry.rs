use crate::{
    AiStupidLevelSource, ArcAgiSource, ArtificialAnalysisSource, BfclSource, GsoSource,
    LiveCodeBenchSource, LmArenaSource, McpAtlasSource, OpenRouterSource, OverridesSource,
    SonarSource, Source, SweAtlasQnaSource, SweAtlasRefactoringSource, SweAtlasTestWritingSource,
    SweBenchProSource, SweBenchSource, SweRebenchSource, TerminalBenchSource,
};

pub fn registry() -> Vec<Box<dyn Source>> {
    vec![
        Box::new(OpenRouterSource),
        Box::new(LmArenaSource),
        Box::new(ArtificialAnalysisSource),
        Box::new(AiStupidLevelSource),
        Box::new(SweBenchSource),
        Box::new(SweBenchProSource),
        Box::new(SweAtlasQnaSource),
        Box::new(SweAtlasTestWritingSource),
        Box::new(SweAtlasRefactoringSource),
        Box::new(SweRebenchSource),
        Box::new(LiveCodeBenchSource),
        Box::new(GsoSource),
        Box::new(BfclSource),
        Box::new(TerminalBenchSource),
        Box::new(SonarSource),
        Box::new(McpAtlasSource),
        Box::new(ArcAgiSource),
        Box::new(OverridesSource::default()),
    ]
}
