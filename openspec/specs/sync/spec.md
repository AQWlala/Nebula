# 多设备同步 行为契约

> **领域**: sync
> **状态**: 当前系统行为 (v2.3.0)
> **最后更新**: 2026-07-11

## 概述

多设备同步模块实现 Nebula 跨设备的数据同步,采用 CRDT(LWW 合并)解决冲突,通过 E2EE 双棘轮加密保证传输安全,支持 QR 码设备配对与中继客户端传输。`DeviceManager` 持久化已配对设备列表。

## Requirements

### Requirement: CRDT 同步
The system SHALL synchronize data across devices using CRDT (Last-Writer-Wins) merge semantics.
- `CrdtEngine` 实现 LWW(Last-Writer-Wins)合并语义
- `CrdtVersion`:版本向量,包含时间戳与设备 ID
- `CrdtMergeResult`:合并结果(accepted / rejected / conflict)
- `FieldChange`:字段级变更追踪
- 整版本级合并(`merge_lww`):swarm agent 通常产出完整条目,字段级合并收益有限
- 字段级合并(`merge_fields`):可选,显式调用 `merge_remote_fields`
- `CrdtOpLog` + `CrdtOpLogEntry`:操作日志持久化到 SQLite(migration 022)
- `CrdtOpStats`:操作统计

#### Scenario: LWW 合并
- **WHEN** 设备 A 和设备 B 同时修改同一条记忆
- **THEN** CrdtEngine 比较版本向量,时间戳更晚的(Last-Writer)胜出
- **AND** 合并结果记录到 CrdtOpLog

#### Scenario: 操作日志持久化
- **WHEN** 任一设备执行 CRDT 操作(本地修改或远端合并)
- **THEN** 操作记录写入 `CrdtOpLog`(migration 022 表)
- **AND** `CrdtOpStats` 累计操作统计

### Requirement: E2EE 双棘轮
The system SHALL encrypt all cross-device traffic with Double Ratchet (DH ratchet + KDF chain ratchet) for forward secrecy.
- 密钥交换:X25519 ECDH,每设备持有长期密钥对,公钥在 QR 配对时交换
- Double Ratchet(v2 信封):
  - DH 棘轮:收到对端新 DH 公钥时生成新密钥对,ECDH 输出经 KDF_RK 更新根密钥
  - KDF 链棘轮:链密钥经 KDF_CK 派生消息密钥,每条消息用唯一密钥后丢弃
- AEAD:AES-256-GCM,每条消息新鲜 12 字节 nonce
- v1 信封(单棘轮)向后兼容:自环回(peer == local)使用 v1
- `E2eeIdentity` / `E2eePublicIdentity`:设备身份与公钥身份
- `EncryptedEnvelope`:加密信封(版本 / dh / nonce / ct)
- `SessionKey`:会话密钥管理
- 重放防护:接收方追踪 "last seen seq",拒绝重复

#### Scenario: 前向保密
- **WHEN** 设备 A 的长期私钥在 100 条消息后被泄露
- **THEN** Double Ratchet 保证泄露前的所有旧消息无法解密
- **AND** 因为每条消息使用独立密钥,密钥用后即弃

#### Scenario: 自环回兼容
- **WHEN** 设备向自己发送消息(peer == local)
- **THEN** 使用 v1 信封(单棘轮)以保持兼容
- **AND** 跨设备新消息使用 v2(双棘轮)

### Requirement: 设备配对
The system SHALL pair devices via QR code exchange (offer → answer flow).
- `pairing` 模块提供 QR 码配对流程
- `PairingOffer` / `PairingAnswer`:配对请求与响应
- `PairingState` / `PairingStage`:配对状态机
- `PAIRING_VERSION`:配对协议版本
- 序列化:`offer_to_qr_string` / `offer_from_qr_string` / `answer_to_qr_string` / `answer_from_qr_string`
- 配对时交换 X25519 公钥,建立共享密钥
- 配对完成后设备持久化到 `DeviceManager`

#### Scenario: QR 码配对流程
- **WHEN** 设备 A 生成配对 offer 并显示为 QR 码
- **THEN** 设备 B 扫描 QR 码,解析为 `PairingOffer`
- **AND** 设备 B 生成 `PairingAnswer` 并显示为 QR 码
- **AND** 设备 A 扫描 answer,完成 X25519 密钥交换
- **AND** 两设备建立共享会话密钥

### Requirement: 密钥库
The system SHALL securely store private keys via the KeyVault abstraction.
- `KeyVault` 模块提供私钥安全存储抽象
- X25519 长期私钥存储于 OS 原生密钥链(keychain 模块)
- 私钥永不离开设备,不写入明文文件
- 会话密钥(SessionKey)在内存中管理,进程退出后清除

#### Scenario: 私钥安全存储
- **WHEN** 设备首次启动生成 X25519 长期密钥对
- **THEN** 私钥通过 KeyVault 存入 OS 原生密钥链
- **AND** 公钥用于 QR 配对时交换
- **AND** 私钥不写入任何明文文件

### Requirement: 中继客户端
The system SHALL exchange encrypted envelopes via a relay client for cross-device transport.
- `relay_client` 模块:中继客户端,通过共享 inbox 交换加密信封
- `transport` 模块:`LocalTransport` 提供本地 inbox 读写
  - `send_sealed`:发送加密信封到对端 inbox
  - `recv_all_unsealed`:读取并解密本端 inbox 所有消息
- `InboxMessage`:inbox 消息封装
- 中继服务器只看到加密密文,无法解密内容(E2EE 保证)

#### Scenario: 中继传输
- **WHEN** 设备 A 向设备 B 发送一条记忆更新
- **THEN** 设备 A 用设备 B 的公钥加密为 `EncryptedEnvelope`
- **AND** 通过 relay_client 发送到中继服务器的共享 inbox
- **AND** 设备 B 的 `recv_all_unsealed` 读取并解密
- **AND** 中继服务器只看到密文,无法解密

### Requirement: DeviceManager 持久化
The system SHALL persist paired devices via DeviceManager with revocation support.
- `DeviceManager` 管理已配对设备列表
- `PairedDevice`:设备 ID / 名称 / 公钥 / 配对时间 / 最后同步时间
- `DeviceRevokeResult`:设备撤销结果
- 撤销设备:移除配对关系,清除会话密钥
- 持久化到 SQLite

#### Scenario: 设备撤销
- **WHEN** 用户在设置中撤销一台已配对设备
- **THEN** `DeviceManager` 移除该设备的 `PairedDevice` 记录
- **AND** 清除与该设备的会话密钥
- **AND** 返回 `DeviceRevokeResult` 确认撤销
- **AND** 该设备后续无法再同步数据
