//! Decodificación de descriptores USB segura y acotada.
//!
//! Valida longitudes, offsets y tamaños. Nunca sigue punteros externos ni
//! asume longitudes confiables.

use super::{ErrorUsb, MAX_DESCRIPTORES_BYTES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TipoDescriptor {
    Dispositivo = 1,
    Configuracion = 2,
    String = 3,
    Interfaz = 4,
    Endpoint = 5,
    Hub = 29,
    Hid = 33,
}

#[derive(Clone, Copy, Debug)]
pub struct Cabecera {
    pub b_length: u8,
    pub b_descriptor_type: u8,
}

pub fn leer_cabecera(buf: &[u8], offset: usize) -> Result<Cabecera, ErrorUsb> {
    if buf.len().saturating_sub(offset) < 2 {
        return Err(ErrorUsb::Truncado);
    }
    let b_length = buf[offset];
    let b_descriptor_type = buf[offset + 1];
    if b_length < 2 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    Ok(Cabecera {
        b_length,
        b_descriptor_type,
    })
}

pub fn validar_tramo(buf: &[u8], offset: usize, len: usize) -> Result<(), ErrorUsb> {
    if len > MAX_DESCRIPTORES_BYTES {
        return Err(ErrorUsb::Desborde);
    }
    let fin = offset.checked_add(len).ok_or(ErrorUsb::Desborde)?;
    if fin > buf.len() {
        return Err(ErrorUsb::Truncado);
    }
    Ok(())
}

pub fn leer_u16_le(buf: &[u8], offset: usize) -> Result<u16, ErrorUsb> {
    validar_tramo(buf, offset, 2)?;
    Ok(u16::from_le_bytes([buf[offset], buf[offset + 1]]))
}

pub fn leer_u16_be(_buf: &[u8], _offset: usize) -> Result<u16, ErrorUsb> {
    Err(ErrorUsb::NoSoportado)
}

#[derive(Clone, Copy, Debug)]
pub struct DescDispositivo {
    pub bcd_usb: u16,
    pub b_device_class: u8,
    pub b_device_sub_class: u8,
    pub b_device_protocol: u8,
    pub b_max_packet_size0: u8,
    pub id_vendor: u16,
    pub id_product: u16,
    pub bcd_device: u16,
    pub i_manufacturer: u8,
    pub i_product: u8,
    pub i_serial_number: u8,
    pub b_num_configurations: u8,
}

pub fn parsear_dispositivo(buf: &[u8]) -> Result<DescDispositivo, ErrorUsb> {
    validar_tramo(buf, 0, 18)?;
    let cab = leer_cabecera(buf, 0)?;
    if cab.b_length != 18 || cab.b_descriptor_type != TipoDescriptor::Dispositivo as u8 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    Ok(DescDispositivo {
        bcd_usb: leer_u16_le(buf, 2)?,
        b_device_class: buf[4],
        b_device_sub_class: buf[5],
        b_device_protocol: buf[6],
        b_max_packet_size0: buf[7],
        id_vendor: leer_u16_le(buf, 8)?,
        id_product: leer_u16_le(buf, 10)?,
        bcd_device: leer_u16_le(buf, 12)?,
        i_manufacturer: buf[14],
        i_product: buf[15],
        i_serial_number: buf[16],
        b_num_configurations: buf[17],
    })
}

#[derive(Clone, Copy, Debug)]
pub struct DescConfiguracion {
    pub w_total_length: u16,
    pub b_num_interfaces: u8,
    pub b_configuration_value: u8,
    pub bm_attributes: u8,
    pub b_max_power: u8,
}

pub fn parsear_configuracion(buf: &[u8], offset: usize) -> Result<DescConfiguracion, ErrorUsb> {
    validar_tramo(buf, offset, 9)?;
    let cab = leer_cabecera(buf, offset)?;
    if cab.b_length != 9 || cab.b_descriptor_type != TipoDescriptor::Configuracion as u8 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    Ok(DescConfiguracion {
        w_total_length: leer_u16_le(buf, offset + 2)?,
        b_num_interfaces: buf[offset + 4],
        b_configuration_value: buf[offset + 5],
        bm_attributes: buf[offset + 6],
        b_max_power: buf[offset + 7],
    })
}

#[derive(Clone, Copy, Debug)]
pub struct DescInterfaz {
    pub b_interface_number: u8,
    pub b_alternate_setting: u8,
    pub b_num_endpoints: u8,
    pub b_interface_class: u8,
    pub b_interface_sub_class: u8,
    pub b_interface_protocol: u8,
}

pub fn parsear_interfaz(buf: &[u8], offset: usize) -> Result<DescInterfaz, ErrorUsb> {
    validar_tramo(buf, offset, 9)?;
    let cab = leer_cabecera(buf, offset)?;
    if cab.b_length != 9 || cab.b_descriptor_type != TipoDescriptor::Interfaz as u8 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    Ok(DescInterfaz {
        b_interface_number: buf[offset + 2],
        b_alternate_setting: buf[offset + 3],
        b_num_endpoints: buf[offset + 4],
        b_interface_class: buf[offset + 5],
        b_interface_sub_class: buf[offset + 6],
        b_interface_protocol: buf[offset + 7],
    })
}

#[derive(Clone, Copy, Debug)]
pub struct DescEndpoint {
    pub b_endpoint_address: u8,
    pub bm_attributes: u8,
    pub w_max_packet_size: u16,
    pub b_interval: u8,
}

pub fn parsear_endpoint(buf: &[u8], offset: usize) -> Result<DescEndpoint, ErrorUsb> {
    validar_tramo(buf, offset, 7)?;
    let cab = leer_cabecera(buf, offset)?;
    if cab.b_length != 7 || cab.b_descriptor_type != TipoDescriptor::Endpoint as u8 {
        return Err(ErrorUsb::LongitudInvalida);
    }
    Ok(DescEndpoint {
        b_endpoint_address: buf[offset + 2],
        bm_attributes: buf[offset + 3],
        w_max_packet_size: leer_u16_le(buf, offset + 4)?,
        b_interval: buf[offset + 6],
    })
}

pub fn recorrer_descriptores<F>(buf: &[u8], mut f: F) -> Result<(), ErrorUsb>
where
    F: FnMut(usize, Cabecera) -> bool,
{
    let mut i = 0;
    while i + 1 < buf.len() {
        let cab = leer_cabecera(buf, i)?;
        if cab.b_length == 0 {
            return Err(ErrorUsb::LongitudInvalida);
        }
        let seguir = f(i, cab);
        if !seguir {
            break;
        }
        let ni = i
            .checked_add(cab.b_length as usize)
            .ok_or(ErrorUsb::Desborde)?;
        if ni <= i {
            return Err(ErrorUsb::Desborde);
        }
        i = ni;
        if i > MAX_DESCRIPTORES_BYTES {
            return Err(ErrorUsb::Desborde);
        }
    }
    Ok(())
}
