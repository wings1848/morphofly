// TEMP PROBE - 将被最终实现替换
fn _probe_send() {
    fn assert_send<T: Send>() {}
    assert_send::<cpal::Stream>();
    assert_send::<rodio::MixerDeviceSink>();
    assert_send::<rodio::Player>();
}
