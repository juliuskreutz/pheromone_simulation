struct Species {
    color: vec3<f32>,
    amount: u32,
    move_speed: f32,
    turn_speed: f32,
    sensor_angle: f32,
    sensor_offset: f32,
    sensor_size: i32,
    decay_rate: f32,
    diffuse_rate: f32,
    like_index: u32,
    like_length: u32,
    hate_index: u32,
    hate_length: u32,
}

struct Agent {
    position: vec2<f32>,
    angle: f32,
    species: u32,
}

@group(0)
@binding(0)
var<uniform> width: u32;

@group(0)
@binding(1)
var<uniform> height: u32;

@group(0)
@binding(2)
var<uniform> x: u32;

@group(0)
@binding(3)
var<uniform> y: u32;

@group(0)
@binding(4)
var texture: texture_storage_2d<rgba32float, read_write>;

@group(0)
@binding(5)
var<storage> species: array<Species>;

@group(0)
@binding(6)
var<storage> relations: array<u32>;

@group(0)
@binding(7)
var<storage, read_write> agents: array<Agent>;

@group(0)
@binding(8)
var<storage, read_write> weights: array<f32>;

@group(0)
@binding(9)
var<storage> time_delta: f32;

fn hash(state: u32) -> u32 {
    var hash = state;
    hash ^= 2747636419u;
    hash *= 2654435769u;
    hash ^= hash >> 16u;
    hash *= 2654435769u;
    hash ^= hash >> 16u;
    hash *= 2654435769u;
    return hash;
}

fn sense(i: u32, dir: f32) -> f32 {
    let agent = agents[i];
    let spec = species[agent.species];

    let sensor_angle = spec.sensor_angle * dir;
    let sensor_offset = spec.sensor_offset;
    let sensor_size = spec.sensor_size;

    let angle = agent.angle + sensor_angle;
    let direction = vec2<f32>(cos(angle), sin(angle));
    let position = agent.position + direction * sensor_offset;

    var sum = 0.0;

    for (var offset_x = -sensor_size; offset_x <= sensor_size; offset_x++) {
        for (var offset_y = -sensor_size; offset_y <= sensor_size; offset_y++) {
            let pos = vec2<u32>(position + vec2<f32>(f32(offset_x), f32(offset_y)));

            if (pos.x >= 0u && pos.x < width && pos.y >= 0u && pos.y < height) {

                let like_end = spec.like_index + spec.like_length;
                for (var like_index = spec.like_index; like_index < like_end; like_index++) {
                    let liked_species = relations[like_index];
                    let species_length = arrayLength(&species);
                    let weight_index = (pos.x * species_length + pos.y * width * species_length) + liked_species;
                    sum += weights[weight_index];
                }

                let hate_end = spec.hate_index + spec.hate_length;
                for (var hate_index = spec.hate_index; hate_index < hate_end; hate_index++) {
                    let hated_species = relations[hate_index];
                    let species_length = arrayLength(&species);
                    let weight_index = (pos.x * species_length + pos.y * width * species_length) + hated_species;
                    sum -= weights[weight_index];
                }
            }
        }
    }

    return sum;
}

@compute
@workgroup_size(1, 1, 1)
fn main_1(@builtin(global_invocation_id) id: vec3<u32>) {
    let i = id.x + id.y * x + id.z * x * y;

    if (i >= arrayLength(&agents)) {
        return;
    }

    let agent = agents[i];
    let spec = species[agent.species];
    let position = agent.position;
    let angle = agent.angle;

    let move_speed = spec.move_speed;
    let turn_speed = spec.turn_speed * 2.0 * 3.1415;

    let weight_forward = sense(i, 0.0);
    let weight_left = sense(i, 1.0);
    let weight_right = sense(i, -1.0);

    let random = f32(hash(u32(position.y) * width + u32(position.x) + hash(i))) / 4294967295.0;

    if (weight_forward > weight_left && weight_forward > weight_right) {
        agents[i].angle += 0.0;
    } else if (weight_forward < weight_left && weight_forward < weight_right) {
        agents[i].angle += (random - 0.5) * 2.0 * turn_speed * time_delta;
    } else if (weight_right > weight_left) {
        agents[i].angle -= random * turn_speed * time_delta;
    } else if (weight_left > weight_right) {
        agents[i].angle += random * turn_speed * time_delta;
    }

    let direction = vec2<f32>(cos(angle), sin(angle));
    var new_position = position + direction * time_delta * move_speed;

    if (new_position.x < 0.0) {
        new_position.x = f32(width) - 1.0;
    } else if (new_position.x >= f32(width)) {
        new_position.x = 0.0;
    }

    if (new_position.y < 0.0) {
        new_position.y = f32(height) - 1.0;
    } else if (new_position.y >= f32(height)) {
        new_position.y = 0.0;
    }

    let species_length = arrayLength(&species);
    let weight_index = (u32(new_position.x) * species_length + u32(new_position.y) * width * species_length) + agent.species;
    weights[weight_index] = 1.0;

    agents[i].position = new_position;
}

@compute
@workgroup_size(1)
fn main_2(@builtin(global_invocation_id) id: vec3<u32>) {
    let length = arrayLength(&species);

    var sum = vec3<f32>(0.0, 0.0, 0.0);
    var amount = 0;
    for (var i = 0u; i < length; i++) {
        let species_length = arrayLength(&species);
        let weight_index = (id.x * species_length + id.y * width * species_length) + i;
        let weight = weights[weight_index];

        if (weight != 0.0) {
            sum += species[i].color * weights[weight_index];
            amount++;
        }
    }

    let amount = f32(amount);
    sum /= vec3<f32>(amount, amount, amount);
    textureStore(texture, vec2<i32>(id.xy), vec4<f32>(sum, 1.0));
}

@compute
@workgroup_size(1)
fn main_3(@builtin(global_invocation_id) id: vec3<u32>) {
    let length = arrayLength(&species);

    for (var i = 0u; i < length; i++) {
        let spec = species[i];
        let decay_rate = spec.decay_rate * time_delta;
        let diffuse_rate = spec.diffuse_rate * time_delta;

        var sum = 0.0;
        for (var offset_x = -1; offset_x <= 1; offset_x++) {
            for (var offset_y = -1; offset_y <= 1; offset_y++) {
                let sample_x = min(i32(width) - 1, max(0, i32(id.x) + offset_x));
                let sample_y = min(i32(height) - 1, max(0, i32(id.y) + offset_y));

                let species_length = arrayLength(&species);
                let weight_index = (u32(sample_x) * species_length + u32(sample_y) * width * species_length) + i;
                sum += weights[weight_index];
            }
        }

        let species_length = arrayLength(&species);
        let weight_index = (id.x * species_length + id.y * width * species_length) + i;
        let weight = weights[weight_index];

        sum /= 9.0;
        sum = weight * (1.0 - diffuse_rate) + sum * diffuse_rate;


        weights[weight_index] = max(0.0, sum - decay_rate);
    }
}
