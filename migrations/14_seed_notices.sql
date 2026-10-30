/**
 * ============================================================================
 * ██████╗ ██╗  ██╗ ██████╗     ███████╗████████╗██╗   ██╗██████╗ ██╗ ██████╗
 * ██╔══██╗██║  ██║██╔═══██╗    ██╔════╝╚══██╔══╝██║   ██║██╔══██╗██║██╔═══██╗
 * ██████╔╝███████║██║   ██║    ███████╗   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██╔══██╗██╔══██║██║   ██║    ╚════██║   ██║   ██║   ██║██║  ██║██║██║   ██║
 * ██║  ██║██║  ██║╚██████╔╝    ███████║   ██║   ╚██████╔╝██████╔╝██║╚██████╔╝
 * ╚═╝  ╚═╝╚═╝  ╚═╝ ╚═════╝     ╚══════╝   ╚═╝    ╚═════╝ ╚═════╝ ╚═╝ ╚═════╝
 * https://rho.studio/
 * ============================================================================
 * File:        migrations/14_seed_notices.sql
 * Author:      Alexis Tercero
 * Email:       alexis.tercero@rho.studio
 * Date:        2026-10-30
 * ============================================================================
 * Migration 14: Seed Initial Privacy Notices
 * ============================================================================
 * Populates the initial legal texts required for LFPDPPP compliance.
 * The `content_sha256` is dynamically generated using pgcrypto to guarantee
 * it matches the content and passes the `protect_consent_notice()` trigger.
 *
 * This unblocks the KYC flow and the GET /api/v1/consents/notices endpoint
 * on fresh environments (DEV, TEST, CI).
 * ============================================================================
*/
-- 1. General Privacy Notice
INSERT INTO consent_notices (
    purpose_code,
    locale,
    version,
    content,
    content_sha256,
    status,
    published_at
)
VALUES (
    'privacy',
    'es-MX',
    '1.0',
    'Aviso de Privacidad Integral de Rho Studio Fintech. Los datos personales que recabamos son utilizados para la creación, gestión y seguridad de su cuenta, así como para el cumplimiento de obligaciones legales. Puede consultar el aviso completo en nuestra aplicación.',
    encode(digest('Aviso de Privacidad Integral de Rho Studio Fintech. Los datos personales que recabamos son utilizados para la creación, gestión y seguridad de su cuenta, así como para el cumplimiento de obligaciones legales. Puede consultar el aviso completo en nuestra aplicación.', 'sha256'), 'hex'),
    'published',
    now()
)
ON CONFLICT (purpose_code, locale, version) DO NOTHING;

-- 2. KYC Verification Notice (Required for Epic F12)
INSERT INTO consent_notices (
    purpose_code,
    locale,
    version,
    content,
    content_sha256,
    status,
    published_at
)
VALUES (
    'kyc_verification',
    'es-MX',
    '1.0',
    'Aviso de Privacidad para Verificación de Identidad. Para cumplir con la Ley Fintech y prevenir el lavado de dinero (LFPIORPI), requerimos recabar y verificar sus datos financieros y patrimoniales (RFC, CURP, INE). Estos datos serán tratados con estricta confidencialidad y solo para los fines descritos.',
    encode(digest('Aviso de Privacidad para Verificación de Identidad. Para cumplir con la Ley Fintech y prevenir el lavado de dinero (LFPIORPI), requerimos recabar y verificar sus datos financieros y patrimoniales (RFC, CURP, INE). Estos datos serán tratados con estricta confidencialidad y solo para los fines descritos.', 'sha256'), 'hex'),
    'published',
    now()
)
ON CONFLICT (purpose_code, locale, version) DO NOTHING;