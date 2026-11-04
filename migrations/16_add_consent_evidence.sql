-- ============================================================================
-- ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
-- ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
-- ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
-- ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
-- ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
-- https://rho.studio/
-- ============================================================================
-- File:        16_add_consent_evidence.sql
-- Author:      Alexis Tercero
-- Email:       alexis.tercero@rho.studio
-- Date:        2026-11-04
-- ============================================================================
-- Migration 16: Add consent evidence tracking
-- ============================================================================
-- Adds columns to the `customer_consent_events` table to track the IP address, 
-- user agent, and consent type (express or tacit) for each consent event. 
-- This information is useful for auditing and compliance purposes. #42
--
ALTER TABLE customer_consent_events
ADD COLUMN ip_address INET,
ADD COLUMN user_agent TEXT,
ADD COLUMN consent_type VARCHAR(16) CHECK (consent_type IN ('express', 'tacit'));