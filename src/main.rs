use as5600::As5600;
use bosch_bme680::*;
use core::cell::RefCell;
use embedded_hal_bus::i2c;
use esp_idf_svc::hal::{
    delay::{Ets, FreeRtos},
    gpio::*,
    i2c::{I2cConfig, I2cDriver},
    peripherals::Peripherals,
    units::Hertz,
};
use log::info;
use mqtt::publish_wifi_data;
use weather_station::*;
mod mqtt;

fn main() {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    esp_idf_svc::log::set_target_level("weather-station", log::LevelFilter::Debug).unwrap();

    //SETUP
    let p = Peripherals::take().unwrap();
    let i2c = I2cDriver::new(
        p.i2c0,
        p.pins.gpio21,
        p.pins.gpio22,
        &I2cConfig::new().baudrate(Hertz(100_000)),
    )
    .expect("fail creating i2c");
    let i2c_bus = RefCell::new(i2c);
    let mut delay_prov = Ets;

    //PIN_ANEMO
    let mut pin_anemo = PinDriver::input(p.pins.gpio27).unwrap();
    let mut pin_rain = PinDriver::input(p.pins.gpio25).unwrap();
    set_intterupt(&mut pin_rain, &mut pin_anemo)
        .unwrap_or_else(|e| log::error!("An Error occured setting the interrupts: {e}"));

    //WIFI
    let mut wifi = wifi_init(p.modem).unwrap();
    connect_wifi(&mut wifi).expect("couldn't connect to wifi");

    //I2C PERIPHERALS
    let mut as5600 = As5600::new(i2c::RefCellDevice::new(&i2c_bus));
    let mut bme = Bme680::new(
        i2c::RefCellDevice::new(&i2c_bus),
        DeviceAddress::Secondary,
        &mut delay_prov,
        &bosch_bme680::Configuration::default(),
        20,
    )
    .expect("Fail initiating bme");

    //MQTT LOOP
    let (mut mqtt_cli, mut mqtt_conn) =
        mqtt::mqtt_create(CONFIG.broker_url, CONFIG.mqtt_user).expect("Fail creating mqtt client");

    std::thread::scope(|s| {
        info!("Starting MQTT client");

        //Creates a thread that will keep alive the connection between broker and client
        std::thread::Builder::new()
            .stack_size(6000)
            .spawn_scoped(s, move || {
                info!("MQTT Listening for messages");
                while let Ok(event) = mqtt_conn.next() {
                    info!("[Queue] Event: {}", event.payload());
                }
                info!("Connection closed");
            })
            .unwrap(); //TODO: Try to get rid of this, try to disconnect raspberry

        loop {
            check_rain_flag(&mut pin_rain);
            check_rotation_flag(&mut pin_anemo);

            if check_time_passed() {
                let wind_direction = get_wind_direction(&mut as5600);
                let bme_readings = get_bme_readings(&mut bme);

                //TODO: check rain flag, check anemo_flag

                publish_wifi_data(&mut mqtt_cli, &mut wifi);
                mqtt::publish_bme_data(&mut mqtt_cli, bme_readings);
                mqtt::publish_anemo_data(&mut mqtt_cli, wind_direction);
                mqtt::publish_rain_data(&mut mqtt_cli);
            }

            FreeRtos::delay_ms(200);
        }
    })
}
