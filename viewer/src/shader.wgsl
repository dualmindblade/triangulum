// Phase 0 shader: camera-relative vertices, lambert sun + ambient, gamma out.

struct Globals {
    view_proj: mat4x4<f32>,
    // inverse view-projection for the sky pass (camera at the origin, so an
    // unprojected far-plane point IS the view ray)
    inv_view_proj: mat4x4<f32>,
    sun_dir: vec4<f32>,
    // xyz = unit direction to the moon; drives the night moon disc + moonlight
    moon_dir: vec4<f32>,
    // disc cut out of the heightfield where voxel chunks own the ground:
    // xyz = center relative to camera (km), w = radius in km (0 = off)
    hole: vec4<f32>,
    hole_up: vec4<f32>,
    // xyz = camera radial up, w = camera height above the sphere (km)
    sky: vec4<f32>,
    // xyz = planet center relative to the camera (km), w = voxel patch lift
    center: vec4<f32>,
    // x = number of active torch lights, y = time (s, wraps hourly)
    misc: vec4<f32>,
    // weather at the camera: x cloud cover, y precip, z snow fraction,
    // w air temp C (999 = weather off, disables ground responses)
    weather: vec4<f32>,
    // xyz = cloud-domain drift (synoptic advection), w = overcast sun floor
    weather2: vec4<f32>,
    // x rain darkening cap, y full-dusting cold depth (C),
    // z cloud shell altitude (km), w shell fade-out camera height (km)
    weather3: vec4<f32>,
    // xyz = instantaneous wind (km/s, camera-relative) for particle slant
    weather4: vec4<f32>,
    // premultiplied cave-noise seeds (low 32 bits of
    // (seed+K).wrapping_mul(0x9E37_79B1)) for the karst breach hint:
    // x = region gate (+40961), y = tube n1 (+31337), z = tube n2 (+51413)
    karst: vec4<u32>,
    // placed-torch point lights: xyz camera-relative (km), w intensity
    lights: array<vec4<f32>, 16>,
};
struct Tile {
    // xyz: tile origin minus camera position, in km (computed in f64 on the
    // CPU). w: 1 for heightfield tiles (subject to the hole cut), 0 for
    // voxel chunks.
    offset: vec4<f32>,
    // x, y = geomorph band start/end distances (km); vertices slide toward
    // the parent triangle across [x, y] to retire the visible LOD pop.
    // Zero for voxel chunks and level-0 tiles.
    morph: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;
@group(0) @binding(1) var<uniform> tile: Tile;

struct VsIn {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) color: vec3<f32>,
    // rgb = water color, a = wetness flag on mesh tiles / cave-darkness
    // factor on voxel chunks
    @location(3) water: vec4<f32>,
    // radial delta (km) to the parent triangle's interpolated height here
    @location(4) morph_dh: f32,
    // wetness the parent triangle actually interpolates here (the thread
    // width is level-dependent, so unmorphed paint pops at every tile split)
    @location(5) morph_wet: f32,
    // 1.0 on a sea/lake water-surface vertex: the heightfield hole does NOT
    // cut these, so the mesh water plane stays under the voxel patch and
    // backs any perimeter crack with water instead of the (void) sky.
    @location(6) wflag: f32,
    // signed water-minus-ground delta (km): its interpolated zero crossing
    // IS the shoreline, stepped per fragment (-1 = no standing water)
    @location(7) shore: f32,
};
struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) color: vec3<f32>,
    @location(2) dist_km: f32,
    // camera-relative position (xyz, km) + the tile/chunk flag (w)
    @location(3) rel_flag: vec4<f32>,
    @location(4) water: vec4<f32>,
    @location(5) wflag: f32,
    @location(6) shore: f32,
};

// Karst twin of noise.rs GRAD (generated from noise_grad.rs - the
// planetgen parity table; do not hand-edit).
const KGRAD = array<vec3<f32>, 256>(
    vec3<f32>(-0.276032466, -0.241130319, 0.93040972),
    vec3<f32>(0.991480371, 0.04028209, 0.123871007),
    vec3<f32>(-0.743505967, -0.585643918, -0.322831347),
    vec3<f32>(0.392402975, 0.286014546, -0.874194249),
    vec3<f32>(0.80440911, 0.455421838, -0.381466818),
    vec3<f32>(0.357206137, -0.492564879, 0.793589073),
    vec3<f32>(-0.0841678571, -0.766706561, 0.636456456),
    vec3<f32>(-0.892438004, -0.00305680084, 0.451159689),
    vec3<f32>(-0.661071603, -0.733827554, -0.156465513),
    vec3<f32>(0.56451794, 0.0716952637, 0.822301213),
    vec3<f32>(-0.561246956, -0.716753187, 0.413843839),
    vec3<f32>(-0.467297399, -0.259865656, -0.845046142),
    vec3<f32>(0.900654415, -0.359400011, 0.244240161),
    vec3<f32>(-0.334292427, 0.885581167, 0.322481891),
    vec3<f32>(-0.919026082, -0.196467252, -0.341747976),
    vec3<f32>(-0.247348586, 0.955824385, 0.158803092),
    vec3<f32>(0.834210297, -0.32992149, -0.441865353),
    vec3<f32>(-0.962683697, 0.212652103, -0.167389316),
    vec3<f32>(0.92626537, -0.229556355, -0.29889186),
    vec3<f32>(0.879159774, -0.222478711, 0.421403979),
    vec3<f32>(-0.0948048811, 0.446197852, 0.889898596),
    vec3<f32>(0.413218212, -0.91021099, 0.0276886958),
    vec3<f32>(-0.191944698, -0.718217561, 0.66882043),
    vec3<f32>(0.527572594, -0.782164072, -0.331491362),
    vec3<f32>(-0.158135314, 0.0870774729, -0.983570402),
    vec3<f32>(0.926047755, -0.358307248, -0.118538904),
    vec3<f32>(0.844739235, 0.416316312, 0.336298014),
    vec3<f32>(-0.0178833604, 0.723249941, 0.69035477),
    vec3<f32>(-0.522158047, 0.429313923, -0.736912837),
    vec3<f32>(0.0434986213, -0.447932188, -0.893008749),
    vec3<f32>(-0.463207253, 0.879862867, -0.106209117),
    vec3<f32>(0.646764584, 0.392698928, 0.653821936),
    vec3<f32>(0.224490004, -0.415077172, -0.881654796),
    vec3<f32>(-0.962134816, -0.198129669, 0.187193031),
    vec3<f32>(0.930964332, -0.355814074, 0.081864268),
    vec3<f32>(-0.961892279, 0.0853028185, 0.259781973),
    vec3<f32>(0.811261317, -0.407150137, 0.419623453),
    vec3<f32>(0.703017124, -0.63026949, -0.32943481),
    vec3<f32>(0.386658151, -0.339026756, 0.857645809),
    vec3<f32>(0.432356706, 0.871866684, -0.230035137),
    vec3<f32>(0.850235093, -0.0288870647, 0.525609955),
    vec3<f32>(-0.531380491, 0.24650952, -0.810473831),
    vec3<f32>(-0.0347171227, -0.744163163, 0.667095127),
    vec3<f32>(-0.912043835, 0.409322006, -0.0251304338),
    vec3<f32>(-0.691338759, -0.555773466, 0.461699659),
    vec3<f32>(-0.980809233, -0.180984577, 0.0725109038),
    vec3<f32>(-0.90234931, -0.430246921, 0.0255599378),
    vec3<f32>(0.052947524, 0.952511984, -0.299862435),
    vec3<f32>(-0.173202039, 0.953852632, -0.245288016),
    vec3<f32>(-0.497829308, 0.864162178, 0.0734146402),
    vec3<f32>(0.916452405, -0.113076699, 0.383834143),
    vec3<f32>(0.431640852, 0.865067449, 0.255625671),
    vec3<f32>(-0.677675807, 0.699383382, -0.227196797),
    vec3<f32>(0.000282057278, 0.404560826, 0.914511049),
    vec3<f32>(-0.0460834508, -0.9987779, -0.0178611759),
    vec3<f32>(-0.928116314, -0.318852143, -0.192180692),
    vec3<f32>(-0.0617358404, -0.971678354, -0.228100551),
    vec3<f32>(0.246749832, 0.797304255, -0.550836133),
    vec3<f32>(0.214014395, -0.150437887, -0.965176813),
    vec3<f32>(0.279872651, -0.0757977559, 0.957040229),
    vec3<f32>(0.98400028, -0.0494065353, 0.171179563),
    vec3<f32>(0.0305211621, -0.790295299, 0.611965521),
    vec3<f32>(0.744553932, 0.0758574625, 0.663238334),
    vec3<f32>(0.892359215, 0.141799109, 0.428471753),
    vec3<f32>(-0.0981353238, -0.623043691, 0.776006455),
    vec3<f32>(-0.917712563, 0.113344754, -0.380731687),
    vec3<f32>(0.0369537423, 0.835935546, 0.547582125),
    vec3<f32>(-0.304622294, -0.0887924602, -0.948325449),
    vec3<f32>(0.711129038, 0.700984175, 0.0540062776),
    vec3<f32>(0.63742031, -0.683235686, 0.356208289),
    vec3<f32>(-0.981787691, 0.186412004, -0.0366537146),
    vec3<f32>(-0.273268774, 0.889222372, -0.366889289),
    vec3<f32>(0.142881906, 0.123286768, -0.982031127),
    vec3<f32>(0.137983549, 0.435042061, 0.88977466),
    vec3<f32>(0.939563426, -0.342368658, 0.00206644081),
    vec3<f32>(0.181358537, 0.850869678, 0.493082013),
    vec3<f32>(0.0893046888, -0.995881789, -0.0156248435),
    vec3<f32>(-0.0088380908, -0.98859122, -0.15036385),
    vec3<f32>(0.337899178, -0.460840491, 0.820640108),
    vec3<f32>(-0.0132037926, 0.283430951, 0.958901745),
    vec3<f32>(-0.216379523, 0.265055479, 0.939641152),
    vec3<f32>(0.973343725, -0.200225914, -0.111855159),
    vec3<f32>(0.49413138, 0.46619012, -0.733826241),
    vec3<f32>(0.0220358097, 0.211775185, -0.977069953),
    vec3<f32>(0.16688953, 0.179411352, 0.96951506),
    vec3<f32>(0.186365445, -0.223437212, -0.956735979),
    vec3<f32>(0.670117255, 0.359557738, 0.64935437),
    vec3<f32>(-0.696773289, 0.496068882, -0.518095211),
    vec3<f32>(0.821370067, -0.394040665, 0.412411405),
    vec3<f32>(-0.227416005, 0.528559262, 0.81786739),
    vec3<f32>(-0.0544659821, 0.262332601, -0.963439185),
    vec3<f32>(-0.599383849, 0.694259341, -0.398425614),
    vec3<f32>(-0.197928924, 0.211071236, -0.957221539),
    vec3<f32>(-0.413940726, 0.892985024, 0.176722443),
    vec3<f32>(-0.171905371, 0.290684031, 0.94124988),
    vec3<f32>(0.787023181, 0.603850762, 0.126328023),
    vec3<f32>(0.816089442, -0.493482163, 0.300787928),
    vec3<f32>(0.36208818, -0.866695775, 0.343118905),
    vec3<f32>(-0.7301804, -0.389383525, -0.561441941),
    vec3<f32>(0.771671336, -0.577609247, -0.266253464),
    vec3<f32>(-0.405038302, -0.876353853, -0.260668175),
    vec3<f32>(0.211696974, -0.440611001, -0.872379698),
    vec3<f32>(0.140663869, 0.447924442, -0.882936787),
    vec3<f32>(0.590045122, 0.12112019, 0.798233458),
    vec3<f32>(0.0940180276, 0.692486036, -0.715278757),
    vec3<f32>(-0.0296743626, 0.997852276, 0.0583974968),
    vec3<f32>(0.764896974, 0.636412263, 0.0995592774),
    vec3<f32>(-0.778775318, 0.532720543, 0.331236816),
    vec3<f32>(-0.33581108, 0.525011498, 0.782044657),
    vec3<f32>(0.404411734, 0.702125233, -0.586064252),
    vec3<f32>(0.188152016, 0.302334981, -0.934447633),
    vec3<f32>(0.0315604529, 0.593961092, 0.803874467),
    vec3<f32>(-0.691569822, 0.710371983, -0.130777777),
    vec3<f32>(0.293605458, -0.538721226, 0.789667826),
    vec3<f32>(-0.809976437, 0.408424932, 0.420864878),
    vec3<f32>(0.640349956, 0.759475054, -0.1146716),
    vec3<f32>(-0.0927443115, -0.762660485, 0.640115206),
    vec3<f32>(0.930610355, -0.266706119, -0.250663547),
    vec3<f32>(0.291364254, -0.949559689, -0.115945107),
    vec3<f32>(-0.750244828, 0.362027305, 0.553234967),
    vec3<f32>(0.243608163, 0.959128138, 0.143973183),
    vec3<f32>(0.0551593311, 0.00556649844, 0.998462048),
    vec3<f32>(-0.686078867, -0.134107112, -0.715060186),
    vec3<f32>(-0.456262394, 0.161381437, -0.875088944),
    vec3<f32>(0.547235689, -0.325181154, 0.771226502),
    vec3<f32>(0.560259019, 0.684030459, -0.467131847),
    vec3<f32>(-0.507229031, -0.80475747, -0.308357139),
    vec3<f32>(-0.405511944, 0.683705436, 0.606718171),
    vec3<f32>(0.849951023, 0.0842158564, 0.520087443),
    vec3<f32>(-0.962719717, 0.0434632105, 0.26698632),
    vec3<f32>(0.6891646, 0.0302225643, 0.723974275),
    vec3<f32>(-0.493805958, 0.477678019, -0.7266219),
    vec3<f32>(-0.352194522, -0.619114589, -0.701894682),
    vec3<f32>(-0.842369793, -0.371691813, 0.390202931),
    vec3<f32>(-0.133158175, 0.889284051, -0.437541744),
    vec3<f32>(-0.296630401, 0.209082782, 0.931823371),
    vec3<f32>(0.985119923, 0.170787567, 0.0192443331),
    vec3<f32>(0.447586202, -0.0614819557, 0.892124745),
    vec3<f32>(-0.386723643, 0.861379876, -0.32934713),
    vec3<f32>(0.297881548, 0.0826213155, 0.951020663),
    vec3<f32>(-0.522436196, -0.343142958, -0.780585249),
    vec3<f32>(0.330618443, 0.365252239, -0.870219654),
    vec3<f32>(-0.0621210476, -0.945362758, 0.320047234),
    vec3<f32>(-0.567057813, 0.609271189, -0.554286979),
    vec3<f32>(0.798538414, 0.329296786, 0.503884935),
    vec3<f32>(0.351783621, 0.715478403, 0.603604952),
    vec3<f32>(-0.673757458, -0.0360521628, -0.738072577),
    vec3<f32>(-0.182456273, -0.255421717, -0.949457453),
    vec3<f32>(-0.432594955, 0.573826397, 0.695402669),
    vec3<f32>(-0.663323759, -0.485617291, -0.569365821),
    vec3<f32>(-0.397947229, -0.431567606, 0.809560006),
    vec3<f32>(-0.582849286, 0.47062894, 0.662416116),
    vec3<f32>(0.173384965, 0.01214674, 0.984779219),
    vec3<f32>(0.0893690977, 0.160169028, -0.983035628),
    vec3<f32>(0.790482157, 0.424137682, -0.441865575),
    vec3<f32>(0.324454931, -0.292128944, -0.899660868),
    vec3<f32>(0.328304546, -0.25622218, 0.909156928),
    vec3<f32>(0.541503283, 0.0976770899, -0.835005018),
    vec3<f32>(-0.0493433009, -0.997933, -0.041169959),
    vec3<f32>(-0.764958918, -0.392519636, -0.51065271),
    vec3<f32>(-0.577839993, -0.0337919581, 0.815450211),
    vec3<f32>(-0.629590435, -0.759555102, 0.163376655),
    vec3<f32>(-0.961263276, -0.135227293, -0.240180127),
    vec3<f32>(0.563623227, -0.0776193005, 0.822377105),
    vec3<f32>(0.0414185461, 0.676050253, 0.735690532),
    vec3<f32>(0.219209636, 0.974905676, 0.038807963),
    vec3<f32>(-0.809674613, 0.586632351, -0.0170148641),
    vec3<f32>(-0.495498842, 0.75636396, -0.427076641),
    vec3<f32>(0.769278384, 0.524217469, -0.365248974),
    vec3<f32>(0.425023643, -0.255286349, 0.868437553),
    vec3<f32>(-0.794295871, -0.141637985, 0.590789938),
    vec3<f32>(-0.325914011, -0.356948928, 0.875424194),
    vec3<f32>(-0.715950975, -0.169736882, -0.67720277),
    vec3<f32>(0.593339899, -0.528769479, -0.606918943),
    vec3<f32>(-0.896487884, 0.211339549, 0.389416317),
    vec3<f32>(-0.948451912, 0.276360387, -0.155125454),
    vec3<f32>(0.493269829, 0.445979208, 0.746851673),
    vec3<f32>(0.859039836, -0.491263903, -0.143910864),
    vec3<f32>(-0.729847935, 0.540908854, -0.418018663),
    vec3<f32>(0.209031498, -0.181834534, 0.960854846),
    vec3<f32>(0.117162773, -0.634744769, -0.763787904),
    vec3<f32>(-0.298839711, 0.947543321, -0.113386425),
    vec3<f32>(-0.790810525, 0.227376373, 0.568259358),
    vec3<f32>(-0.583718383, 0.811946709, 0.00392313892),
    vec3<f32>(0.51160152, 0.808500417, 0.290845252),
    vec3<f32>(-0.24149277, 0.481271425, 0.842650021),
    vec3<f32>(-0.279860569, 0.952684487, 0.118618419),
    vec3<f32>(0.200600462, -0.775842578, 0.598187051),
    vec3<f32>(0.108970218, 0.983957954, -0.141252387),
    vec3<f32>(-0.361610633, -0.880562269, 0.306345948),
    vec3<f32>(-0.802219789, 0.572256665, -0.170193184),
    vec3<f32>(-0.105614569, -0.600130489, 0.792899085),
    vec3<f32>(0.795024365, 0.601026642, 0.0818732856),
    vec3<f32>(-0.786899314, -0.427478128, 0.445030246),
    vec3<f32>(0.42508356, 0.159933061, -0.890912669),
    vec3<f32>(0.270044675, -0.561459346, -0.782201558),
    vec3<f32>(-0.587715155, -0.595400123, 0.547804336),
    vec3<f32>(0.257361019, 0.461375065, -0.849057333),
    vec3<f32>(-0.375537759, 0.407463184, 0.832433268),
    vec3<f32>(-0.531339941, 0.845929055, 0.0456278467),
    vec3<f32>(-0.545671705, -0.682830069, -0.485783375),
    vec3<f32>(0.367475068, -0.278713962, 0.887288342),
    vec3<f32>(-0.91624388, -0.257187307, -0.30716745),
    vec3<f32>(0.441561174, 0.583170464, -0.681862112),
    vec3<f32>(-0.34009252, 0.528415108, -0.777891092),
    vec3<f32>(-0.979890557, 0.191105196, 0.0573872876),
    vec3<f32>(-0.625995602, 0.31709819, -0.712445257),
    vec3<f32>(0.21450976, -0.0526686213, -0.975300764),
    vec3<f32>(0.745060725, 0.478980638, 0.464178914),
    vec3<f32>(0.0244852159, 0.519895207, 0.85387906),
    vec3<f32>(0.699670919, 0.594563189, 0.396175743),
    vec3<f32>(-0.852537704, -0.250582717, 0.458680461),
    vec3<f32>(0.134106828, 0.89500603, -0.425416932),
    vec3<f32>(0.970360783, -0.166381146, 0.175263415),
    vec3<f32>(-0.969198568, -0.233352219, -0.0787456571),
    vec3<f32>(-0.51404617, -0.515671057, 0.685448682),
    vec3<f32>(-0.507233736, 0.137257116, 0.850808099),
    vec3<f32>(0.159874524, -0.422126176, -0.892328206),
    vec3<f32>(-0.672674459, 0.708940099, 0.211926892),
    vec3<f32>(0.63991845, -0.746945539, -0.180490273),
    vec3<f32>(0.347880643, -0.829322541, -0.437267859),
    vec3<f32>(-0.901118332, 0.351059644, -0.254446219),
    vec3<f32>(-0.597016042, -0.0146101101, -0.802096247),
    vec3<f32>(0.925977992, -0.0315549991, 0.376256614),
    vec3<f32>(0.872485909, 0.0445222042, -0.486606732),
    vec3<f32>(-0.0658411323, 0.765633273, 0.639898927),
    vec3<f32>(-0.808789749, 0.47708446, 0.343874338),
    vec3<f32>(-0.403455262, -0.860233361, 0.311805093),
    vec3<f32>(0.325483351, 0.860366203, -0.392212423),
    vec3<f32>(-0.013876593, -0.366774794, -0.930206262),
    vec3<f32>(0.493826746, 0.149543463, 0.856604867),
    vec3<f32>(-0.88008479, -0.363934748, -0.304962722),
    vec3<f32>(-0.456221127, -0.889066667, -0.0377192834),
    vec3<f32>(-0.305954581, 0.613482815, 0.728032026),
    vec3<f32>(-0.817555204, 0.226670094, 0.529362028),
    vec3<f32>(0.253512351, -0.875994197, 0.410323842),
    vec3<f32>(-0.0125429371, 0.570953698, -0.820886441),
    vec3<f32>(0.731156613, 0.507331164, -0.456097683),
    vec3<f32>(0.432170282, 0.423884706, 0.79595892),
    vec3<f32>(0.555898894, -0.103206168, -0.824818105),
    vec3<f32>(-0.813088974, -0.574960231, 0.0911430348),
    vec3<f32>(-0.974757902, 0.223206386, -0.00509326911),
    vec3<f32>(-0.154445551, -0.927594176, 0.340169983),
    vec3<f32>(0.743178611, -0.643266691, -0.184101918),
    vec3<f32>(-0.194825419, 0.893176392, -0.405313446),
    vec3<f32>(0.307074424, 0.841558455, -0.444392467),
    vec3<f32>(0.945351513, 0.30589831, 0.112857172),
    vec3<f32>(0.22056963, 0.12905243, 0.966796001),
    vec3<f32>(-0.664541243, -0.589779251, -0.458852233),
    vec3<f32>(-0.380935604, -0.0860154076, -0.920591883),
    vec3<f32>(0.189632119, -0.543279607, -0.817855078),
    vec3<f32>(-0.297834738, -0.0965324716, 0.949724145),
    vec3<f32>(-0.885106, 0.439976463, 0.151684149),
    vec3<f32>(-0.275089116, -0.279066825, 0.920025916),
    vec3<f32>(-0.843058137, 0.534687665, -0.0579834393),
    vec3<f32>(-0.267546352, 0.73302797, -0.625371046),
);

// Bit-exact u32 twin of noise.rs hash(): every step of the Rust i64
// arithmetic is masked to 32 bits, so u32 wrapping ops reproduce it
// exactly. seedmul = low 32 bits of seed.wrapping_mul(0x9E37_79B1),
// premultiplied on the CPU (globals.karst).
fn khash(ix: i32, iy: i32, iz: i32, seedmul: u32) -> u32 {
    var h: u32 = bitcast<u32>(ix) * 0x8DA6B343u
        + bitcast<u32>(iy) * 0xD8163841u
        + bitcast<u32>(iz) * 0xCB1AB31Fu
        + seedmul;
    h = (h ^ (h >> 13u)) * 0xC2B2AE35u;
    h = h ^ (h >> 16u);
    return h & 255u;
}

// f32 twin of noise.rs gradient_noise: same lattice, same gradients, same
// fade - worst-case f32 drift is millimetres against the 8 m tube bands.
fn kgnoise(p: vec3<f32>, seedmul: u32) -> f32 {
    let pi = floor(p);
    let pf = p - pi;
    let ii = vec3<i32>(pi);
    let f = pf * pf * pf * (pf * (pf * 6.0 - 15.0) + 10.0);
    var total = 0.0;
    for (var dx = 0; dx < 2; dx = dx + 1) {
        let wx = select(1.0 - f.x, f.x, dx == 1);
        for (var dy = 0; dy < 2; dy = dy + 1) {
            let wy = select(1.0 - f.y, f.y, dy == 1);
            for (var dz = 0; dz < 2; dz = dz + 1) {
                let wz = select(1.0 - f.z, f.z, dz == 1);
                let g = KGRAD[khash(ii.x + dx, ii.y + dy, ii.z + dz, seedmul)];
                let d = pf - vec3<f32>(f32(dx), f32(dy), f32(dz));
                total = total + wx * wy * wz * dot(g, d);
            }
        }
    }
    return total * 1.9;
}

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    var rel = in.pos + tile.offset.xyz;
    let d = length(rel);
    var wet = in.water.a;
    if (tile.morph.y > 0.0) {
        // Geomorphing: slide to the parent triangle's height and its actual
        // triangle-interpolated river paint. The scalar radial slide retains
        // only the measured <= 0.13 m residual in the V-6 level-9 probes.
        let m = clamp((d - tile.morph.x) / (tile.morph.y - tile.morph.x), 0.0, 1.0);
        let radial = normalize(rel - globals.center.xyz);
        rel += radial * (in.morph_dh * m);
        wet = mix(in.water.a, in.morph_wet, m);
    }
    if (tile.offset.w < 0.5 && globals.misc.z > 0.0) {
        // voxel chunks: toward the SELECTED patch radius (misc.z) the
        // blocks sink from their lift down flush with the mesh, so the
        // patch ends in a feathered shoreline instead of a floating
        // one-block cliff. Tied to the selected radius, not the hole —
        // the hole tracks built coverage and may lag while streaming.
        let q = rel - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = length(q - globals.hole_up.xyz * vert);
        let sink = smoothstep(globals.misc.z * 0.85, globals.misc.z * 1.06, horiz);
        rel -= globals.hole_up.xyz * (globals.center.w * 1.1 * sink);
    }
    out.clip = globals.view_proj * vec4<f32>(rel, 1.0);
    out.normal = in.normal;
    out.color = in.color;
    out.dist_km = length(rel);
    out.rel_flag = vec4<f32>(rel, tile.offset.w);
    out.water = vec4<f32>(in.water.rgb, wet);
    out.wflag = in.wflag;
    out.shore = in.shore;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    // cut the heightfield away inside the voxel patch: every pixel belongs
    // to exactly one system (blocks own the near disc, the mesh the rest).
    // The vertical slab test keeps far-side geometry out of the cylinder.
    // EXCEPTION: sea/lake water surfaces (wflag) are NOT cut — block water
    // and mesh water are the same surface, so cutting the mesh water opens a
    // see-through crack at the patch boundary (a black void underwater).
    // Leaving the water plane under the patch backs any crack with water;
    // the opaque blocks still draw over it, so nothing double-shows.
    // The exemption needs wflag ~ 1: it interpolates across a triangle, so
    // a MIXED shore triangle (two sea verts + one land vert) kept its
    // wflag>0.5 half alive inside the patch — a floating pale shore-ramp
    // fragment over the block water (photographed at 9.20 114.05). Only
    // all-water triangles (wflag 1 at every vertex) may skip the cut.
    if (in.rel_flag.w > 0.5 && globals.hole.w > 0.0 && in.wflag < 0.98) {
        let q = in.rel_flag.xyz - globals.hole.xyz;
        let vert = dot(q, globals.hole_up.xyz);
        let horiz = q - globals.hole_up.xyz * vert;
        if (abs(vert) < 25.0 && length(horiz) < globals.hole.w) {
            discard;
        }
    }
    // deep tiles (near the voxel patch) resolve rivers: step the wetness
    // for crisp per-pixel water edges. Far tiles can't — a soft wet tint
    // reads as a distant river valley instead of shattered polygons.
    // (Voxel chunks reuse water.a as their cave-darkness factor instead.)
    var wet = 0.0;
    if (in.rel_flag.w > 0.5) {
        wet = in.water.a * 0.75;
        if (in.rel_flag.w > 1.5) {
            wet = step(0.55, in.water.a);
        }
        // per-pixel shorelines (TRANSITIONS.md B, kills V-5): the signed
        // water-minus-ground field interpolates smoothly across triangles;
        // stepping its zero crossing per fragment with derivative AA draws
        // the sea/lake edge at pixel resolution instead of vertex
        // resolution — no more angular lake polygons or orphan blue cells.
        // Rivers keep their own wetness paint (water.a) untouched.
        let e = max(fwidth(in.shore), 1e-5);
        wet = max(wet, smoothstep(-e, e, in.shore));
    }
    // ---- weather on the ground (WEATHER.md Layer 3) ----
    // Dust and darken the GROUND color before the water mix so lakes and
    // rivers stay water. Pixels lapse-colder than the camera's air whiten
    // first (~6.5 C/km), the line broken up by a world-anchored hash so it
    // reads organic instead of contoured; dusting deepens while snow
    // actually falls. Rain soaks the ground toward dark.
    var ground = in.color;
    let up = globals.sky.xyz;
    let day = smoothstep(-0.08, 0.15, dot(globals.sun_dir.xyz, up));
    // ---- shared micro-texture (TRANSITIONS.md A, kills the V-1 disk) ----
    // The far mesh evaluates the same ±10% block-brightness fabric the
    // voxel columns bake, so the patch rim becomes a resolution change
    // instead of a material change. STATISTICS match (amplitude, ~block
    // scale); cell identity cannot — f32 world coordinates carry ~0.5 m of
    // noise at planet radius, so the blocks' exact u64 lattice is out of
    // reach: same fabric, not the same stitches. Fades over ~2 km so the
    // far field keeps today's clean flat reading; blocks skip it (their
    // hash is baked per column+height).
    if (in.rel_flag.w > 0.5) {
        let wp = in.rel_flag.xyz - globals.center.xyz;
        let cell = vec3<i32>(floor(wp * 480.0)); // ~2 m cells (km units)
        let tex = 0.9 + 0.2 * hash3i(cell);
        let tex_fade = exp(-in.dist_km / 1.8);
        ground = ground * mix(1.0, tex, tex_fade);
    }
    // ---- karst breach hint (BUGS.md V-10) ----
    // The mesh evaluates the SAME cave noises the voxel columns carve
    // with (bit-exact u32 lattice hash, kgnoise; seeds in globals.karst),
    // so surface cave mouths stop popping at the patch boundary: where
    // the tube grazes the surface, a pool joins the water pipeline when
    // the water table is near (the shore field within ~4 m of ground),
    // else the ground darkens into a dry pit mouth. Full strength out to
    // the patch rim (misc.z) so the handoff matches the real carved
    // blocks, then fades - a 10 m pit is sub-pixel long before that.
    if (in.rel_flag.w > 0.5 && wet < 0.98) {
        let kfade = 1.0 - smoothstep(globals.misc.z + 0.4, globals.misc.z + 2.4, in.dist_km);
        if (kfade > 0.02) {
            let wp = in.rel_flag.xyz - globals.center.xyz;
            let dirp = normalize(wp);
            let region = kgnoise(dirp * 90.0, globals.karst.x);
            if (region > -0.05) {
                // depth input to the tube field, in metres like col_ctx's
                // ground0 — computed CAMERA-RELATIVELY. length(wp) at
                // planet magnitude is f32-quantized to ~1 m, and every
                // quantum shifts the tube field ~8 m, which rendered as
                // moire scanlines. Expanding |eye + rel| - R around the
                // camera keeps every term small: sky.w is the camera's
                // height, dot(rel, up) the local slope, and the quadratic
                // term the planet's curvature (~0.3 m at 2 km).
                let u_r = normalize(-globals.center.xyz);
                let e_big = length(globals.center.xyz);
                let radial = dot(in.rel_flag.xyz, u_r);
                let d2 = dot(in.rel_flag.xyz, in.rel_flag.xyz);
                let zm = (globals.sky.w + radial + (d2 - radial * radial) / (2.0 * e_big))
                    * 1000.0;
                let a1 = abs(kgnoise(dirp * (90000.0 + zm / 12.0), globals.karst.y));
                if (a1 < 0.085) {
                    let a2 = abs(kgnoise(dirp * (76000.0 + zm / 9.0), globals.karst.z));
                    // blocks carve on a hard |n| < 0.085 — keep the hint
                    // near-binary too (full strength to 0.076, AA band to
                    // the true edge) or pools render thinner than their
                    // block twins
                    let t = (1.0 - smoothstep(0.076, 0.085, a1))
                        * (1.0 - smoothstep(0.076, 0.085, a2))
                        * smoothstep(-0.05, -0.02, region)
                        * kfade;
                    // flooded whenever the shore field carries ANY lake
                    // proximity (sentinel is exactly -5 m): col_ctx floods
                    // caves across a 10 m groundwater band, so a -4 m gate
                    // painted false dry gaps mid-channel
                    if (in.shore > -0.0049) {
                        wet = max(wet, t); // flooded: the water pipeline colors it
                    } else {
                        ground = ground * (1.0 - 0.7 * t); // dry pit mouth
                    }
                }
            }
        }
    }
    if (globals.weather.w < 500.0) {
        let wp = in.rel_flag.xyz - globals.center.xyz;
        let r_sphere = length(globals.center.xyz) - max(globals.sky.w, 0.0);
        let elev_km = length(wp) - r_sphere;
        let t_pix = globals.weather.w - 6.5 * (elev_km - max(globals.sky.w, 0.0));
        let dith = hash31(floor(wp * 40.0)) * 1.8 - 0.9;
        let cold = 1.0 - smoothstep(-globals.weather3.y, 1.0, t_pix + dith);
        let dust = cold * (0.45 + 0.55 * globals.weather.z * globals.weather.y);
        ground = mix(ground, vec3<f32>(0.88, 0.90, 0.94), dust);
        let rain = globals.weather.y * (1.0 - globals.weather.z);
        ground = ground * (1.0 - globals.weather3.x * rain);
    }
    let base = mix(ground, in.water.rgb, clamp(wet, 0.0, 1.0));
    var n = normalize(in.normal);
    // ---- per-pixel normal detail (TRANSITIONS.md: mesh reads flat) ----
    // The voxel patch gets surface relief for free — stepped 1 m column
    // tops shade individually — while the mesh interpolates one normal
    // across 26 m triangles and reads as vinyl. Perturb the mesh normal
    // per ~2 m hash cell (same lattice family as the micro-texture), with
    // the amplitude scaled by SLOPE: flat ground stays smooth exactly like
    // coplanar block tops do, hillsides roughen exactly where the columns
    // step. Water keeps its mirror (×(1−wet)), and the same ~2 km fade as
    // the micro-texture returns the far field to today's clean reading.
    if (in.rel_flag.w > 0.5) {
        let wp = in.rel_flag.xyz - globals.center.xyz;
        let cell = vec3<i32>(floor(wp * 480.0));
        let jit = vec3<f32>(
            hash3i(cell + vec3<i32>(11, 0, 0)),
            hash3i(cell + vec3<i32>(0, 7, 0)),
            hash3i(cell + vec3<i32>(0, 0, 3)),
        ) - 0.5;
        let pup = normalize(wp);
        let slope = 1.0 - dot(n, pup);
        let amp = 0.9 * smoothstep(0.01, 0.20, slope)
            * exp(-in.dist_km / 1.8) * (1.0 - clamp(wet, 0.0, 1.0));
        n = normalize(n + amp * jit);
    }
    let light = max(dot(n, globals.sun_dir.xyz), 0.0);
    let sky_hemi = clamp(0.5 + 0.5 * dot(n, up), 0.0, 1.0);
    // overcast: direct sun dims toward its tunable floor and the ambient
    // flattens a touch — a grey day, not a dark one (night is untouched:
    // the dimming scales with `day`)
    let odim = mix(1.0, globals.weather2.w, globals.weather.x * day);
    let ambient = (0.10 + 0.40 * day * sky_hemi) * mix(1.0, 0.85, globals.weather.x * day);
    // ...and the direct term itself dies with the horizon (* day): the
    // below-horizon sun otherwise kept lighting at FULL coefficient — tree
    // canopy sides facing the set sun glowed all night, opposite the moon
    // ("tree shading is backwards", night photo at 0.626 68.962)
    let sun_coeff = mix(1.0, 0.60, day) * odim * day;
    var c = base * (ambient + sun_coeff * light);
    if (in.rel_flag.w < 0.5) {
        if (in.water.a > 1.5) {
            // emissive block face (torch flames): full-bright, no sun, no
            // dim — breathing a little on a position-hashed phase
            let fl = 0.86 + 0.22 * sin(globals.misc.y * 9.0
                + dot(in.rel_flag.xyz, vec3<f32>(310.0, 470.0, 130.0)));
            return vec4<f32>(in.color * fl, 1.0);
        }
        // cave darkness, applied here (not baked) so the player's torch can
        // push it back: a warm pool of light that reaches ~10 m and only
        // acts where the dark is — daylight terrain is untouched
        let dim = in.water.a;
        let reach = clamp(1.0 - in.dist_km / 0.010, 0.0, 1.0);
        let torch = reach * reach * (1.0 - dim);
        c = c * dim + base * vec3<f32>(1.0, 0.78, 0.48) * torch * 0.85;
    }
    // placed torches: warm point-light pools on blocks and mesh alike —
    // they light the night and push back cave darkness around themselves
    let n_lights = u32(globals.misc.x);
    for (var i = 0u; i < n_lights; i = i + 1u) {
        let lp = globals.lights[i];
        let dist = distance(in.rel_flag.xyz, lp.xyz);
        let a = clamp(1.0 - dist / 0.011, 0.0, 1.0);
        c += base * vec3<f32>(1.0, 0.70, 0.36) * (a * a * lp.w);
    }
    // aerial perspective: distant terrain melts toward the same horizon
    // color the sky pass uses; the effect thins away with camera altitude
    let atm = exp(-max(globals.sky.w, 0.0) / 45.0);
    // moonlight: a cool directional lift plus a faint ambient floor so night
    // terrain reads as moonlit rather than flat black. Present only at night
    // (fades as the sun rises) and only while the moon is above the horizon.
    let moon = globals.moon_dir.xyz;
    let moon_up = smoothstep(0.0, 0.15, dot(moon, globals.sky.xyz));
    let moonlit = max(dot(n, moon), 0.0);
    // sky-shaped night floor: with a flat floor and a LOW moon, vertical
    // step faces toward the moon glowed brighter than the tops around them
    // and terraces read as bright contour stripes (2026-07-08 night shots).
    // Faces that see more sky keep more of the floor, restoring the
    // tops-over-sides hierarchy while the directional term still lets a low
    // moon rake across scarps.
    let hemi_n = 0.5 + 0.5 * dot(n, globals.sky.xyz);
    c += base * vec3<f32>(0.40, 0.50, 0.72)
        * (moonlit * 0.10 + 0.015 + 0.05 * hemi_n) * (1.0 - day) * moon_up;
    let fog = (1.0 - exp(-in.dist_km * 0.0035)) * atm * 0.7;
    c = mix(c, vec3<f32>(0.55, 0.70, 0.88) * (0.15 + 0.85 * day), fog);
    // wading with your eyes under the surface: everything goes water-blue
    if (globals.hole_up.w > 0.5) {
        c = mix(c, vec3<f32>(0.02, 0.07, 0.16), 0.6);
    }
    // target format is sRGB: linear values are converted on write
    return vec4<f32>(c, 1.0);
}

// ------------------------------------------------------------------- sky

struct SkyOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

fn hash31(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(443.897, 441.423, 437.195));
    q += dot(q, q.yzx + 19.19);
    return fract((q.x + q.y) * q.z);
}

// integer-lattice hash (PCG-style mixing): float hashes built on fract()
// lose their low bits at large coordinates — the cloud domain reaches the
// thousands and hash31 there degenerates into straight diagonal shards
fn hash3i(p: vec3<i32>) -> f32 {
    var v = bitcast<vec3<u32>>(p) * vec3<u32>(1664525u, 1013904223u, 2654435769u);
    v.x += v.y * v.z;
    v.y += v.z * v.x;
    v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z;
    return f32(v.x & 0xFFFFFFu) / 16777216.0;
}

// trilinear value noise + a 4-octave fbm — the cloud deck's fabric
fn vnoise(p: vec3<f32>) -> f32 {
    let i = vec3<i32>(floor(p));
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let c000 = hash3i(i);
    let c100 = hash3i(i + vec3<i32>(1, 0, 0));
    let c010 = hash3i(i + vec3<i32>(0, 1, 0));
    let c110 = hash3i(i + vec3<i32>(1, 1, 0));
    let c001 = hash3i(i + vec3<i32>(0, 0, 1));
    let c101 = hash3i(i + vec3<i32>(1, 0, 1));
    let c011 = hash3i(i + vec3<i32>(0, 1, 1));
    let c111 = hash3i(i + vec3<i32>(1, 1, 1));
    let x00 = mix(c000, c100, u.x);
    let x10 = mix(c010, c110, u.x);
    let x01 = mix(c001, c101, u.x);
    let x11 = mix(c011, c111, u.x);
    return mix(mix(x00, x10, u.y), mix(x01, x11, u.y), u.z);
}

fn cloud_fbm(p: vec3<f32>) -> f32 {
    var v = 0.0;
    var amp = 0.5;
    var q = p;
    for (var i = 0; i < 4; i = i + 1) {
        v += amp * vnoise(q);
        q = q * 2.13 + vec3<f32>(31.7, 17.3, 51.1);
        amp *= 0.5;
    }
    return v;
}

// procedural star field: hash the view direction into cells on a cube
// lattice; a sparse subset of cells hold one star each, rendered as a tiny
// smooth disc with hashed brightness/temperature. Purely directional, so
// the stars are fixed to the sky.
fn stars(dir: vec3<f32>) -> vec3<f32> {
    let sp = dir * 220.0;
    let cell = floor(sp);
    let h = hash31(cell);
    if (h < 0.92) {
        return vec3<f32>(0.0);
    }
    // star position jittered inside the cell; distance to it in cell units
    let jitter = vec3<f32>(hash31(cell + 7.1), hash31(cell + 13.7), hash31(cell + 29.3));
    let d = length(fract(sp) - clamp(jitter, vec3<f32>(0.15), vec3<f32>(0.85)));
    let bright = (h - 0.92) / 0.08; // 0..1, few bright ones
    let disc = smoothstep(0.10 + 0.08 * bright, 0.0, d);
    // temperature tint: cool white to warm white
    let warm = hash31(cell + 3.3);
    let tint = mix(vec3<f32>(0.75, 0.82, 1.0), vec3<f32>(1.0, 0.92, 0.78), warm);
    return tint * disc * (0.25 + 1.6 * bright * bright);
}

@vertex
fn vs_sky(@builtin(vertex_index) vi: u32) -> SkyOut {
    var out: SkyOut;
    let xy = vec2<f32>(f32((vi << 1u) & 2u), f32(vi & 2u)) * 2.0 - 1.0;
    // reversed-Z: depth 0.0 is infinity, so GreaterEqual only wins on
    // untouched (cleared) pixels
    out.clip = vec4<f32>(xy, 0.0, 1.0);
    out.ndc = xy;
    return out;
}

@fragment
fn fs_sky(in: SkyOut) -> @location(0) vec4<f32> {
    // unproject the NEAR plane (reversed-Z ndc z = 1): the camera sits at
    // the origin of this space, so the near-plane point IS the ray
    // direction, and the near column of the inverse VP is numerically
    // clean. (Unprojecting the far plane cancelled to ~1/far — below f32
    // rounding noise once the near plane shrank close to the ground, which
    // flipped rays and blacked out the whole sky under ~8 m altitude.)
    let p = globals.inv_view_proj * vec4<f32>(in.ndc, 1.0, 1.0);
    let dir = normalize(p.xyz / p.w);
    let up = globals.sky.xyz;
    let sun = globals.sun_dir.xyz;
    let h = dot(dir, up);
    let sunh = dot(sun, up);
    let day = smoothstep(-0.08, 0.15, sunh);
    // day gradient: bright horizon band to deeper zenith blue
    let zen = vec3<f32>(0.10, 0.28, 0.62);
    let hor = vec3<f32>(0.55, 0.70, 0.88);
    var c = mix(hor, zen, pow(clamp(h, 0.0, 1.0), 0.55)) * (0.03 + 0.97 * day);
    // heavy weather greys the whole vault, not just where the deck draws:
    // the blue between clouds desaturates and sinks toward overcast murk
    let wcover = globals.weather.x;
    let murk = wcover * (0.35 + 0.45 * globals.weather.y);
    c = mix(c, vec3<f32>(0.52, 0.56, 0.62) * (0.05 + 0.95 * day), murk * day);
    // a low sun warms the sky around it
    let toward = pow(max(dot(dir, sun), 0.0), 6.0);
    let low = 1.0 - smoothstep(0.05, 0.35, sunh);
    c = mix(c, vec3<f32>(0.95, 0.55, 0.30), toward * low * 0.55 * day);
    // sun disc + glow
    let d = dot(dir, sun);
    c += vec3<f32>(1.0, 0.96, 0.86)
        * (smoothstep(0.99965, 0.99996, d) * 3.0 + pow(max(d, 0.0), 800.0) * 0.5) * day;
    // below the horizon line, fade toward space-dark
    c *= smoothstep(-0.10, 0.03, h);
    // the atmosphere thins away with altitude: space is black
    let atm = exp(-max(globals.sky.w, 0.0) / 45.0);
    c *= atm;
    // limb glow: from orbit, rays that graze past the planet pass through
    // its atmosphere shell — a thin lit rim hugging the dark disc. The ray
    // is measured by its closest approach to the planet center; the glow
    // hugs the surface radius and shows only on the sunlit side.
    if (atm < 0.85) {
        let ctr = globals.center.xyz;
        let along = dot(ctr, dir);
        if (along > 0.0) {
            let r_planet = length(ctr) - max(globals.sky.w, 0.0);
            let cp = dir * along; // closest point on the ray to the center
            let b = length(cp - ctr); // miss distance from planet center
            let n = normalize(cp - ctr);
            let lit = smoothstep(-0.15, 0.35, dot(n, sun));
            let shell = exp(-max(b - r_planet, 0.0) / 20.0)
                * smoothstep(r_planet * 0.6, r_planet, b);
            c += vec3<f32>(0.30, 0.50, 0.90) * shell * lit * (1.0 - atm) * 0.8;
        }
    }
    // the moon: a phase-lit sphere opposite the sun. A ray within the moon's
    // angular radius reconstructs the sphere normal (center faces us, limb is
    // tangential) and lights it by the sun for a real terminator; the dark
    // side keeps a faint earthshine. It fades out in daylight and below the
    // horizon, and a soft halo rings it.
    let moon = globals.moon_dir.xyz;
    let moon_vis = (1.0 - 0.9 * day) * smoothstep(-0.06, 0.06, dot(moon, up));
    if (moon_vis > 0.001) {
        let dm = dot(dir, moon);
        let ang = acos(clamp(dm, -1.0, 1.0));
        let R = 0.021; // angular radius (rad, ~1.2 deg — reads as a moon)
        if (ang < R) {
            let off = normalize(dir - moon * dm);
            let t = ang / R; // 0 at center, 1 at the limb
            let nrm = normalize(moon * cos(t * 1.5707963) + off * sin(t * 1.5707963));
            let lit = max(dot(nrm, sun), 0.0);
            // faint surface mottling (maria) so it isn't a flat disc
            let mott = 0.86 + 0.14 * hash31(floor(off * 70.0));
            let disc = smoothstep(1.0, 0.90, t);
            c += vec3<f32>(0.86, 0.89, 0.97) * mott * disc
                * (0.05 + 0.95 * lit) * moon_vis;
        }
        // tight halo hugging the disc (not a broad blob)
        c += vec3<f32>(0.55, 0.62, 0.80) * pow(max(dm, 0.0), 2600.0) * 0.30 * moon_vis;
    }
    // ---- W1 cloud deck (WEATHER.md): a wind-scrolled noise shell a few
    // km up. Drawn after the sun/moon (clouds cover them) and before the
    // stars (which dim behind lit cloud via sky_lum). Fades out entirely
    // by weather3.w camera altitude: W1 renders NO clouds from space — the
    // capped-opacity orbital layer is W3's job, so the "never obscure the
    // ground from orbit" constraint holds trivially for now.
    let cover = globals.weather.x;
    let deck_fade =
        1.0 - smoothstep(globals.weather3.z, globals.weather3.w, max(globals.sky.w, 0.0));
    if (cover > 0.004 && deck_fade > 0.003) {
        let ctr = globals.center.xyz;
        let r_shell = length(ctr) - max(globals.sky.w, 0.0) + globals.weather3.z;
        let b2 = dot(ctr, dir);
        let qd = dot(ctr, ctr) - r_shell * r_shell; // < 0 while under the deck
        let disc2 = b2 * b2 - qd;
        if (disc2 > 0.0 && qd < 0.0) {
            let t_hit = b2 + sqrt(disc2);
            let sdir = normalize(dir * t_hit - ctr); // deck point on its sphere
            // the deck scrolls with the same synoptic drift the weather
            // field advects by — the storm you see IS the storm you get
            let den = cloud_fbm((sdir - globals.weather2.xyz) * 900.0);
            let a_thr = 0.68 - 0.40 * cover;
            var alpha = smoothstep(a_thr, a_thr + 0.18, den);
            // grazing rays pack more deck toward the horizon, then the
            // deck fades below it and thins away to space with the air
            alpha = min(alpha * (1.0 + 0.7 * (1.0 - clamp(h, 0.0, 1.0))), 1.0);
            alpha = alpha * smoothstep(-0.02, 0.08, h) * deck_fade * atm;
            // bright lit tops; heavy rain bellies go slate; a low sun warms
            // the deck the way it warms the sky
            let heavy = clamp(globals.weather.y * 0.9 + cover * 0.45, 0.0, 1.0);
            var ccol = mix(
                vec3<f32>(0.96, 0.97, 1.00),
                vec3<f32>(0.36, 0.39, 0.46),
                clamp(heavy * (0.35 + 0.85 * den), 0.0, 1.0),
            );
            ccol = ccol * (0.10 + 0.95 * day);
            ccol = mix(ccol, vec3<f32>(0.98, 0.62, 0.38), toward * low * 0.4 * day * (1.0 - heavy));
            c = mix(c, ccol, alpha);
        }
    }
    // stars own the dark: night ground and open space alike. They dim away
    // wherever the sky itself has light, so daylight hides them.
    let sky_lum = dot(c, vec3<f32>(0.35, 0.45, 0.20));
    c += stars(dir) * (1.0 - smoothstep(0.01, 0.18, sky_lum));
    if (globals.hole_up.w > 0.5) {
        c = mix(c, vec3<f32>(0.02, 0.07, 0.16), 0.6);
    }
    return vec4<f32>(c, 1.0);
}

// --------------------------------------------------------- precipitation

struct PrecipOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) kind: f32, // 0 = rain streak, 1 = flake
    @location(2) alpha: f32,
};

// integer hash for particle identity — stable at any instance count
fn hash1u(n: u32) -> f32 {
    var x = n * 1664525u + 1013904223u;
    x ^= x >> 16u;
    x *= 2246822519u;
    x ^= x >> 13u;
    return f32(x & 0xFFFFFFu) / 16777216.0;
}

// Instanced rain/snow around the camera: identity is hashed from the
// instance index, motion is a pure function of the (wrapping) frame clock,
// so captures reproduce exactly. Rain falls as wind-slanted streaks; snow
// drifts down as swaying flakes. The volume rides with the camera (W1;
// world-anchoring at f32 precision on a 6371 km sphere jitters worse than
// the ride-along reads — revisit in W2 with camera-local anchoring).
@vertex
fn vs_precip(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> PrecipOut {
    var out: PrecipOut;
    let up = globals.sky.xyz;
    let t = globals.misc.y;
    let h0 = hash1u(ii * 3u + 0u);
    let h1 = hash1u(ii * 3u + 1u);
    let h2 = hash1u(ii * 3u + 2u);
    let snow = select(0.0, 1.0, h2 < globals.weather.z);
    let box_r = 0.045; // km around the camera
    let box_h = 0.038;
    var e1 = cross(up, vec3<f32>(0.0, 0.0, 1.0));
    if (dot(e1, e1) < 0.01) {
        e1 = cross(up, vec3<f32>(0.0, 1.0, 0.0));
    }
    e1 = normalize(e1);
    let e2 = normalize(cross(up, e1));
    let speed = mix(0.0095, 0.0016, snow); // km/s straight-down component
    let phase = fract(h1 - t * speed / box_h);
    let sway = (vnoise(vec3<f32>(t * 0.6 + h0 * 23.0, h1 * 17.0, h2 * 11.0)) - 0.5)
        * mix(0.0004, 0.0035, snow);
    let center = e1 * ((h0 - 0.5) * 2.0 * box_r + sway)
        + e2 * ((fract(h2 * 7.31) - 0.5) * 2.0 * box_r)
        + up * ((phase - 0.5) * 2.0 * box_h);
    // fall direction bends with the wind; streaks align to it
    let fall = normalize(-up * speed + globals.weather4.xyz * 0.4);
    let to_p = center / max(length(center), 0.0015);
    // a particle straight below a down-looking camera has to_p parallel to
    // fall: cross() collapses and normalize() exploded the quad into a
    // ~100 m garbage slab (photographed at 9.20 114.05 in heavy snowfall,
    // pitch -82). Fall back to a horizontal frame axis when degenerate.
    var cr = cross(to_p, fall);
    if (dot(cr, cr) < 1e-6) {
        cr = cross(fall, e1);
    }
    let across = normalize(cr);
    let half_len = mix(0.00040, 0.00006, snow);
    let half_wid = mix(0.000040, 0.00006, snow);
    // vi 0..5 -> two triangles of a quad
    var cid = vi;
    if (vi == 3u) { cid = 2u; }
    if (vi == 4u) { cid = 1u; }
    if (vi == 5u) { cid = 3u; }
    let cx = f32(cid & 1u) * 2.0 - 1.0;
    let cy = f32(cid >> 1u) * 2.0 - 1.0;
    let pos = center + across * (cx * half_wid) + fall * (cy * half_len);
    out.clip = globals.view_proj * vec4<f32>(pos, 1.0);
    out.uv = vec2<f32>(cx, cy);
    out.kind = snow;
    let d = length(center);
    let fade = smoothstep(0.0018, 0.0060, d) * (1.0 - smoothstep(box_r * 0.7, box_r, d));
    out.alpha = fade * mix(0.16, 0.85, snow);
    return out;
}

@fragment
fn fs_precip(in: PrecipOut) -> @location(0) vec4<f32> {
    var a = in.alpha;
    if (in.kind > 0.5) {
        // soft round flake
        a *= 1.0 - smoothstep(0.45, 1.0, length(in.uv));
    } else {
        // streak with soft ends and edges
        a *= (1.0 - smoothstep(0.55, 1.0, abs(in.uv.y))) * (1.0 - abs(in.uv.x) * 0.45);
    }
    let day = smoothstep(-0.08, 0.15, dot(globals.sun_dir.xyz, globals.sky.xyz));
    let col = mix(vec3<f32>(0.72, 0.78, 0.88), vec3<f32>(0.97, 0.98, 1.00), in.kind)
        * (0.30 + 0.70 * day);
    return vec4<f32>(col, a);
}
