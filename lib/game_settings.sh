#@lib shellcheck shell=bash
#@lib GameUserSettings/Game.ini genereren en grafische presets — wordt door ark-manager.sh ge-sourced; enkel functies, geen top-level acties.
#═══════════════════════════════════════════════════════════════════════════════
#  GAME SETTINGS GENERATION
#═══════════════════════════════════════════════════════════════════════════════

create_optimized_game_settings() {
    local map_dir="$1"
    local preset="${2:-high}"
    local ini_file="$map_dir/GameUserSettings.ini"

    [[ -f "$map_dir/map.conf" ]] && write_conf_value "$map_dir/map.conf" "GraphicsPreset" "$preset"

    local xp taming harvest respawn mating baby hatch
    xp=$(read_conf_value "$DEFAULTS_CONF" "XPMultiplier" "1.0")
    taming=$(read_conf_value "$DEFAULTS_CONF" "TamingSpeedMultiplier" "1.0")
    harvest=$(read_conf_value "$DEFAULTS_CONF" "HarvestAmountMultiplier" "1.0")
    respawn=$(read_conf_value "$DEFAULTS_CONF" "ResourceRespawnPeriodMultiplier" "1.0")
    mating=$(read_conf_value "$DEFAULTS_CONF" "MatingIntervalMultiplier" "1.0")
    baby=$(read_conf_value "$DEFAULTS_CONF" "BabyMatureSpeedMultiplier" "1.0")
    hatch=$(read_conf_value "$DEFAULTS_CONF" "EggHatchSpeedMultiplier" "1.0")
    local show_loc tp cross
    show_loc=$(read_conf_value "$DEFAULTS_CONF" "ShowMapPlayerLocation" "True")
    tp=$(read_conf_value "$DEFAULTS_CONF" "AllowThirdPersonPlayer" "True")
    cross=$(read_conf_value "$DEFAULTS_CONF" "ServerCrosshair" "True")
    local admin_pw server_pw rcon_port map_name max_players
    admin_pw=$(read_conf_value "$map_dir/map.conf" "AdminPassword" "")
    server_pw=$(read_conf_value "$map_dir/map.conf" "ServerPassword" "")
    rcon_port=$(read_conf_value "$map_dir/map.conf" "RCONPort" "27020")
    map_name=$(read_conf_value "$map_dir/map.conf" "DisplayName" "ARK")
    max_players=$(read_conf_value "$map_dir/map.conf" "MaxPlayers" "70")

    cat > "$ini_file" <<GUSEOF
[ServerSettings]
ShowMapPlayerLocation=$show_loc
AllowThirdPersonPlayer=$tp
ServerCrosshair=$cross
ServerPassword=$server_pw
ServerAdminPassword=$admin_pw
RCONEnabled=True
RCONPort=$rcon_port
TheMaxStructuresInRange=10500
OxygenSwimSpeedStatMultiplier=1
StructurePreventResourceRadiusMultiplier=1
AlwaysAllowStructurePickup=True
StructurePickupTimeAfterPlacement=30
StructurePickupHoldDuration=0.5
AllowHideDamageSourceFromLogs=True
KickIdlePlayersPeriod=3600
PerPlatformMaxStructuresMultiplier=1
AutoSavePeriodMinutes=15
MaxTamedDinos=5000
MaxTamedDinos_SoftTameLimit=5000
ItemStackSizeMultiplier=1
RCONServerGameLogBuffer=600
AllowHitMarkers=True
XPMultiplier=$xp
TamingSpeedMultiplier=$taming
HarvestAmountMultiplier=$harvest
ResourcesRespawnPeriodMultiplier=$respawn
MatingIntervalMultiplier=$mating
BabyMatureSpeedMultiplier=$baby
EggHatchSpeedMultiplier=$hatch

[ScalabilityGroups]
sg.ResolutionQuality=3
sg.ViewDistanceQuality=3
sg.AntiAliasingQuality=3
sg.ShadowQuality=3
sg.GlobalIlluminationQuality=3
sg.ReflectionQuality=3
sg.PostProcessQuality=3
sg.TextureQuality=3
sg.EffectsQuality=3
sg.FoliageQuality=3
sg.ShadingQuality=3
sg.LandscapeQuality=3

[/Script/ShooterGame.ShooterGameUserSettings]
AdvancedGraphicsQuality=3
MasterAudioVolume=0
SFXAudioVolume=0
MusicAudioVolume=0
bFilmGrain=False
bUserMotionBlur=False
bUseDFAO=True
bUseSSAO=True
bDisableBloom=False
bUseDistanceFieldAmbientOcclusion=True
bHighQualityAnisotropicFiltering=True
bEnableFootstepDecals=True
bEnableFootstepParticles=True
bEnableFluidInteraction=True
bDisableHLOD=False
bLowQualityVFX=False
PreventDetailGraphics=False
GroundClutterDensity=100
GroundClutterRadius=10000
HFSQuality=3
LODScalar=1.0
HighQualityMaterials=True
HighQualitySurfaces=True
bHighQualityLODs=True
bExtraLevelStreamingDistance=True
bEnableColorGrading=True
ScreenPercentage=100.000000
ResolutionSizeX=1920
ResolutionSizeY=1080
FrameRateLimit=0.000000
bUseVSync=False
bUseDynamicResolution=False
bUseLowQualityLevelStreaming=False
FoliageInteractionDistance=1.000000
FoliageInteractionDistanceLimit=1.000000
FoliageInteractionQuantityLimit=1.000000
GUI3DWidgetQuality=100.000000
AudioQualityLevel=2
ActiveLingeringWorldTiles=20
ClientNetQuality=3
bDisableShadows=False

[SessionSettings]
SessionName=${map_name}

[/Script/Engine.GameSession]
MaxPlayers=$max_players

[/Script/Engine.GameUserSettings]
bUseDesiredScreenHeight=False
GUSEOF

    chmod 600 "$ini_file" 2>/dev/null || true
}

#═══════════════════════════════════════════════════════════════════════════════
#  GRAPHICS PRESETS
#═══════════════════════════════════════════════════════════════════════════════

apply_graphics_preset() {
    local map="$1" preset="$2"
    local ini="$MAPS_DIR/$map/GameUserSettings.ini"
    [[ ! -f "$ini" ]] && { log_err "GameUserSettings.ini not found for '$map'."; return 1; }

    local sg_val adv_q res_x res_y scr_pct fps_lim
    local b_dfao b_ssao b_bloom b_dist_ao b_hi_aniso b_foot_dec b_foot_part b_fluid
    local b_low_vfx b_prev_det gnd_dens gnd_rad hfs_q lod_s
    local b_hi_mat b_hi_surf b_hi_lod b_ext_stream b_color_grad b_dyn_res b_low_stream
    local fol_dist fol_limit fol_qty gui3d_q aud_q tiles b_dis_shad

    case "$preset" in
        low)
            sg_val=0; adv_q=0; res_x=640; res_y=480; scr_pct="0.100000"; fps_lim="30.000000"
            b_dfao=False; b_ssao=False; b_bloom=True; b_dist_ao=False; b_hi_aniso=False
            b_foot_dec=False; b_foot_part=False; b_fluid=False; b_low_vfx=True; b_prev_det=True
            gnd_dens=0; gnd_rad=0; hfs_q=0; lod_s="0.5"
            b_hi_mat=False; b_hi_surf=False; b_hi_lod=False; b_ext_stream=False; b_color_grad=False
            b_dyn_res=False; b_low_stream=True
            fol_dist="0.010000"; fol_limit="0.100000"; fol_qty="0.100000"
            gui3d_q="0.000000"; aud_q=0; tiles=5; b_dis_shad=True ;;
        medium)
            sg_val=2; adv_q=2; res_x=1280; res_y=720; scr_pct="75.000000"; fps_lim="60.000000"
            b_dfao=False; b_ssao=True; b_bloom=False; b_dist_ao=False; b_hi_aniso=False
            b_foot_dec=True; b_foot_part=True; b_fluid=False; b_low_vfx=False; b_prev_det=False
            gnd_dens=50; gnd_rad=5000; hfs_q=2; lod_s="1.0"
            b_hi_mat=False; b_hi_surf=False; b_hi_lod=False; b_ext_stream=False; b_color_grad=True
            b_dyn_res=False; b_low_stream=False
            fol_dist="0.500000"; fol_limit="1.000000"; fol_qty="0.500000"
            gui3d_q="50.000000"; aud_q=1; tiles=10; b_dis_shad=False ;;
        high)
            sg_val=3; adv_q=3; res_x=1920; res_y=1080; scr_pct="100.000000"; fps_lim="0.000000"
            b_dfao=True; b_ssao=True; b_bloom=False; b_dist_ao=True; b_hi_aniso=True
            b_foot_dec=True; b_foot_part=True; b_fluid=True; b_low_vfx=False; b_prev_det=False
            gnd_dens=100; gnd_rad=10000; hfs_q=3; lod_s="1.0"
            b_hi_mat=True; b_hi_surf=True; b_hi_lod=True; b_ext_stream=True; b_color_grad=True
            b_dyn_res=False; b_low_stream=False
            fol_dist="1.000000"; fol_limit="1.000000"; fol_qty="1.000000"
            gui3d_q="100.000000"; aud_q=2; tiles=20; b_dis_shad=False ;;
        auto)
            sg_val=3; adv_q=3; res_x=1920; res_y=1080; scr_pct="100.000000"; fps_lim="0.000000"
            b_dfao=True; b_ssao=True; b_bloom=False; b_dist_ao=True; b_hi_aniso=True
            b_foot_dec=True; b_foot_part=True; b_fluid=True; b_low_vfx=False; b_prev_det=False
            gnd_dens=100; gnd_rad=10000; hfs_q=3; lod_s="1.0"
            b_hi_mat=True; b_hi_surf=True; b_hi_lod=True; b_ext_stream=True; b_color_grad=True
            b_dyn_res=True; b_low_stream=False
            fol_dist="1.000000"; fol_limit="1.000000"; fol_qty="1.000000"
            gui3d_q="100.000000"; aud_q=2; tiles=20; b_dis_shad=False ;;
        *) log_err "Unknown preset: $preset"; return 1 ;;
    esac

    for key in ResolutionQuality ViewDistanceQuality AntiAliasingQuality ShadowQuality \
               GlobalIlluminationQuality ReflectionQuality PostProcessQuality TextureQuality \
               EffectsQuality FoliageQuality ShadingQuality LandscapeQuality; do
        sed -i "s/^sg\.${key}=.*/sg.${key}=$sg_val/" "$ini"
    done
    sed -i "s/^AdvancedGraphicsQuality=.*/AdvancedGraphicsQuality=$adv_q/" "$ini"
    sed -i "s/^bUseDFAO=.*/bUseDFAO=$b_dfao/" "$ini"
    sed -i "s/^bUseSSAO=.*/bUseSSAO=$b_ssao/" "$ini"
    sed -i "s/^bDisableBloom=.*/bDisableBloom=$b_bloom/" "$ini"
    sed -i "s/^bUseDistanceFieldAmbientOcclusion=.*/bUseDistanceFieldAmbientOcclusion=$b_dist_ao/" "$ini"
    sed -i "s/^bHighQualityAnisotropicFiltering=.*/bHighQualityAnisotropicFiltering=$b_hi_aniso/" "$ini"
    sed -i "s/^bEnableFootstepDecals=.*/bEnableFootstepDecals=$b_foot_dec/" "$ini"
    sed -i "s/^bEnableFootstepParticles=.*/bEnableFootstepParticles=$b_foot_part/" "$ini"
    sed -i "s/^bEnableFluidInteraction=.*/bEnableFluidInteraction=$b_fluid/" "$ini"
    sed -i "s/^bLowQualityVFX=.*/bLowQualityVFX=$b_low_vfx/" "$ini"
    sed -i "s/^PreventDetailGraphics=.*/PreventDetailGraphics=$b_prev_det/" "$ini"
    sed -i "s/^GroundClutterDensity=.*/GroundClutterDensity=$gnd_dens/" "$ini"
    sed -i "s/^GroundClutterRadius=.*/GroundClutterRadius=$gnd_rad/" "$ini"
    sed -i "s/^HFSQuality=.*/HFSQuality=$hfs_q/" "$ini"
    sed -i "s/^LODScalar=.*/LODScalar=$lod_s/" "$ini"
    sed -i "s/^HighQualityMaterials=.*/HighQualityMaterials=$b_hi_mat/" "$ini"
    sed -i "s/^HighQualitySurfaces=.*/HighQualitySurfaces=$b_hi_surf/" "$ini"
    sed -i "s/^bHighQualityLODs=.*/bHighQualityLODs=$b_hi_lod/" "$ini"
    sed -i "s/^bExtraLevelStreamingDistance=.*/bExtraLevelStreamingDistance=$b_ext_stream/" "$ini"
    sed -i "s/^bEnableColorGrading=.*/bEnableColorGrading=$b_color_grad/" "$ini"
    sed -i "s/^ScreenPercentage=.*/ScreenPercentage=$scr_pct/" "$ini"
    sed -i "s/^ResolutionSizeX=.*/ResolutionSizeX=$res_x/" "$ini"
    sed -i "s/^ResolutionSizeY=.*/ResolutionSizeY=$res_y/" "$ini"
    sed -i "s/^FrameRateLimit=.*/FrameRateLimit=$fps_lim/" "$ini"
    sed -i "s/^bUseDynamicResolution=.*/bUseDynamicResolution=$b_dyn_res/" "$ini"
    sed -i "s/^bUseLowQualityLevelStreaming=.*/bUseLowQualityLevelStreaming=$b_low_stream/" "$ini"
    sed -i "s/^FoliageInteractionDistance=.*/FoliageInteractionDistance=$fol_dist/" "$ini"
    sed -i "s/^FoliageInteractionDistanceLimit=.*/FoliageInteractionDistanceLimit=$fol_limit/" "$ini"
    sed -i "s/^FoliageInteractionQuantityLimit=.*/FoliageInteractionQuantityLimit=$fol_qty/" "$ini"
    sed -i "s/^GUI3DWidgetQuality=.*/GUI3DWidgetQuality=$gui3d_q/" "$ini"
    sed -i "s/^AudioQualityLevel=.*/AudioQualityLevel=$aud_q/" "$ini"
    sed -i "s/^ActiveLingeringWorldTiles=.*/ActiveLingeringWorldTiles=$tiles/" "$ini"
    sed -i "s/^bDisableShadows=.*/bDisableShadows=$b_dis_shad/" "$ini"
    [[ -f "$MAPS_DIR/$map/map.conf" ]] && write_conf_value "$MAPS_DIR/$map/map.conf" "GraphicsPreset" "$preset"

    log_ok "Graphics preset '${BLD}$preset${R}${GRN}' applied to ${BLD}$map${R}${GRN}. Restart to take effect."
}

graphics_preset_menu() {
    local map="$1"
    local current; current=$(read_conf_value "$MAPS_DIR/$map/map.conf" "GraphicsPreset" "high")
    clear
    echo ""
    echo "  ${BLD}${MAG}═══ Graphics Preset — ${CYN}$map${MAG} ═══${R}"
    thin_sep
    echo "  Current: ${BLD}${CYN}$current${R}"
    echo ""
    echo "  ${CYN}1${R}) ${RED}Low${R}       ${DIM}— Max performance, 640x480, 30fps cap${R}"
    echo "  ${CYN}2${R}) ${YEL}Medium${R}    ${DIM}— Balanced, 1280x720, 60fps cap${R}"
    echo "  ${CYN}3${R}) ${GRN}High${R}      ${DIM}— Max quality, 1920x1080, unlimited fps${R}"
    echo "  ${CYN}4${R}) ${BLU}Auto${R}      ${DIM}— High + dynamic resolution${R}"
    echo ""
    echo "  ${DIM}0) Back${R}"
    echo ""
    echo -n "  ${WHT}Choice: ${R}"
    read -r choice
    case "$choice" in
        1) apply_graphics_preset "$map" "low" ;;
        2) apply_graphics_preset "$map" "medium" ;;
        3) apply_graphics_preset "$map" "high" ;;
        4) apply_graphics_preset "$map" "auto" ;;
        0|"") return ;;
        *) log_err "Invalid choice." ;;
    esac
    sleep 1
}

