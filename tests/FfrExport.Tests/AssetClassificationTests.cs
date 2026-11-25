using Newtonsoft.Json.Linq;
using Xunit;

namespace FfrExport.Tests;

public sealed class AssetClassificationTests
{
    public static IEnumerable<object[]> AssetClassCases()
    {
        // Export: tables, curves, data assets, input, sequencer, gameplay tags, schema
        yield return ["DataTable", "export", "datatable"];
        yield return ["CompositeDataTable", "export", "datatable"];
        yield return ["StringTable", "export", "datatable"];
        yield return ["CurveTable", "export", "curve"];
        yield return ["CurveFloat", "export", "curve"];
        yield return ["CurveLinearColor", "export", "curve"];
        yield return ["CurveVector", "export", "curve"];
        yield return ["DataAsset", "export", "data_asset"];
        yield return ["PrimaryDataAsset", "export", "data_asset"];
        yield return ["PrimaryAssetLabel", "export", "data_asset"];
        yield return ["CPP_DA_MUCHA_INPUT", "export", "data_asset"];
        yield return ["CPP_DA_MapLandmark", "export", "data_asset"];
        yield return ["CPP_DMI_DA_KeyDisplay", "export", "data_asset"];
        yield return ["DA_Option_KeySetting_InvalidKeys", "export", "data_asset"];
        yield return ["CPP_EU_LocalizedKeyBrushDataAsset", "export", "data_asset"];
        yield return ["CPP_NawatobiDataAsset", "export", "data_asset"];
        yield return ["CPP_DM_EventShortcutData", "export", "data_asset"];
        yield return ["InputAction", "export", "input"];
        yield return ["InputMappingContext", "export", "input"];
        yield return ["LevelSequence", "export", "sequencer"];
        yield return ["GameplayTagTable", "export", "gameplay_tags"];
        yield return ["UserDefinedEnum", "export", "schema"];
        yield return ["UserDefinedStruct", "export", "schema"];

        // Blueprints: CDO + component defaults only
        yield return ["BlueprintGeneratedClass", "export_blueprint_cdo", "blueprint"];
        yield return ["Blueprint", "export_blueprint_cdo", "blueprint"];
        yield return ["WidgetBlueprintGeneratedClass", "export_blueprint_cdo", "widget_blueprint"];
        yield return ["WidgetBlueprint", "export_blueprint_cdo", "widget_blueprint"];

        // Skip levels
        yield return ["World", "skip_level", "level"];

        // Skip images
        yield return ["Texture2D", "skip_image", "image_texture"];
        yield return ["TextureCube", "skip_image", "image_texture"];
        yield return ["TextureRenderTarget2D", "skip_image", "image_texture"];
        yield return ["VolumeTexture", "skip_image", "image_texture"];
        yield return ["MediaTexture", "skip_image", "image_texture"];
        yield return ["ManaTexture", "skip_image", "image_texture"];
        yield return ["CurveLinearColorAtlas", "skip_image", "image_texture"];
        yield return ["PaperSprite", "skip_image", "image_paper2d"];
        yield return ["PaperFlipbook", "skip_image", "image_paper2d"];
        yield return ["Ss6Project", "skip_image", "image_spritestudio"];

        // Skip large / non-AI binaries
        yield return ["MapBuildDataRegistry", "skip_binary", "level_build_data"];
        yield return ["StaticMesh", "skip_binary", "mesh_geometry"];
        yield return ["SkeletalMesh", "skip_binary", "mesh_geometry"];
        yield return ["FoliageType_InstancedStaticMesh", "skip_binary", "mesh_geometry"];
        yield return ["AnimSequence", "skip_binary", "animation_rigging"];
        yield return ["AnimBlueprintGeneratedClass", "skip_binary", "animation_rigging"];
        yield return ["Skeleton", "skip_binary", "animation_rigging"];
        yield return ["PhysicsAsset", "skip_binary", "animation_rigging"];
        yield return ["SoundAtomCue", "skip_binary", "audio"];
        yield return ["SoundAtomCueSheet", "skip_binary", "audio"];
        yield return ["SoundWave", "skip_binary", "audio"];
        yield return ["FileManaMovie", "skip_binary", "video"];
        yield return ["FileMediaSource", "skip_binary", "video"];
        yield return ["NiagaraSystem", "skip_binary", "vfx"];
        yield return ["ParticleSystem", "skip_binary", "vfx"];
        yield return ["Material", "skip_binary", "material"];
        yield return ["MaterialInstanceConstant", "skip_binary", "material"];
        yield return ["NNEModelData", "skip_binary", "render_denoiser"];
        yield return ["Font", "skip_binary", "font"];
        yield return ["FontFace", "skip_binary", "font"];

        // Undecided
        yield return ["CPP_MapPhysicalMaterial", "undecided", "undecided"];
        yield return ["CPP_MovieSceneCustomClockSource_Tick", "undecided", "undecided"];
        yield return ["InterchangeGenericAssetsPipeline", "undecided", "undecided"];
        yield return ["UnknownFutureClass", "undecided", "undecided"];
    }

    [Theory]
    [MemberData(nameof(AssetClassCases))]
    public void ClassifyAssetClass_DecidesEveryClassAccurately(
        string className,
        string expectedDecision,
        string expectedCategory)
    {
        var result = Program.ClassifyAssetClass(className);
        Assert.Equal(expectedDecision, result.Decision);
        Assert.Equal(expectedCategory, result.Category);
        Assert.False(string.IsNullOrWhiteSpace(result.Reason));
    }

    [Fact]
    public async Task ExportAssets_ClassifyCli_OutputsExpectedJsonForAllClasses()
    {
        var cases = AssetClassCases()
            .Select(row => ((string)row[0], (string)row[1], (string)row[2]))
            .ToArray();
        var argList = string.Join(",", cases.Select(item => item.Item1));

        var result = await TestHarness.RunCliAsync(["export-assets", "--classify", argList]);
        Assert.Equal(0, result.ExitCode);

        var parsed = JArray.Parse(result.Stdout);
        Assert.Equal(cases.Length, parsed.Count);

        for (var i = 0; i < cases.Length; i++)
        {
            var obj = Assert.IsType<JObject>(parsed[i]);
            Assert.Equal(cases[i].Item1, obj["className"]?.Value<string>());
            Assert.Equal(cases[i].Item2, obj["decision"]?.Value<string>());
            Assert.Equal(cases[i].Item3, obj["category"]?.Value<string>());
            Assert.False(string.IsNullOrWhiteSpace(obj["reason"]?.Value<string>()));
        }
    }

    [Theory]
    [InlineData("FFRS/Config/DefaultGame.ini", "copy_raw", "config_ini")]
    [InlineData("FFRS/FFRS.uproject", "copy_raw", "config_project")]
    [InlineData("FFRS/Plugins/Sample/Sample.uplugin", "copy_raw", "config_project")]
    [InlineData("FFRS/Content/UI/Icon.png", "skip_image", "image_raw")]
    [InlineData("FFRS/Content/Localization/Game/ja/Game.locres", "skip_binary", "localization_handled")]
    [InlineData("FFRS/Content/Textures/T_Sample.ubulk", "skip_binary", "bulk_payload")]
    [InlineData("FFRS/Content/Movies/Intro.usm", "skip_binary", "video_raw")]
    [InlineData("FFRS/Content/Sound/BGM.awb", "skip_binary", "audio_raw")]
    [InlineData("FFRS/Content/Fonts/Main.ufont", "skip_binary", "font_raw")]
    [InlineData("FFRS/AssetRegistry.bin", "skip_binary", "shader_or_registry_bin")]
    [InlineData("FFRS/Content/Other/Unknown.dat", "skip_binary", "other_binary")]
    public void ClassifyNonPackageFile_MatchesExpectedRule(
        string path,
        string expectedDecision,
        string expectedCategory)
    {
        var result = Program.ClassifyNonPackageFile(path);
        Assert.Equal(expectedDecision, result.Decision);
        Assert.Equal(expectedCategory, result.Category);
    }

    [Fact]
    public void PathHelpers_ExtractTopDirectoryAndNormalizeExtension()
    {
        Assert.Equal("FFRS/Content/Datatable", Program.ExtractTopDirectory("FFRS/Content/Datatable/Scenario/DT_A.uasset"));
        Assert.Equal("FFRS/Config", Program.ExtractTopDirectory("FFRS/Config"));
        Assert.Equal("FFRS", Program.ExtractTopDirectory("FFRS"));
        Assert.Equal("(root)", Program.ExtractTopDirectory(""));

        Assert.Equal(".uasset", Program.NormalizeExtension("FFRS/Content/A.UASSET"));
        Assert.Equal("(none)", Program.NormalizeExtension("FFRS/Content/NoExt"));
        Assert.True(Program.IsPackageFile("FFRS/Content/A.uasset"));
        Assert.True(Program.IsPackageFile("FFRS/Content/Map/B.UMAP"));
        Assert.False(Program.IsPackageFile("FFRS/Config/DefaultGame.ini"));
    }
}
