#include "ui/RenderWidget.h"

#include "render/IRenderer.h"

RenderWidget::RenderWidget(QWidget* parent) : QWidget(parent), m_renderer(createRenderer())
{
    setAttribute(Qt::WA_NativeWindow);
    setAttribute(Qt::WA_PaintOnScreen);
    setAttribute(Qt::WA_NoSystemBackground);
}

RenderWidget::~RenderWidget() = default;

QSize RenderWidget::pixelSize() const { return size() * devicePixelRatioF(); }

void RenderWidget::showEvent(QShowEvent*)
{
    if (m_ready)
        return;
    const QSize px = pixelSize();
    m_ready = m_renderer->init(reinterpret_cast<void*>(winId()), px.width(), px.height());
    if (!m_ready)
        qCritical("Renderer initialisation failed");
    else
        m_renderer->loadScene(m_scene);
}

void RenderWidget::setScene(Scene scene)
{
    m_scene = std::move(scene);
    if (m_ready) {
        m_renderer->loadScene(m_scene);
        update();
    }
}

void RenderWidget::resizeEvent(QResizeEvent*)
{
    if (!m_ready)
        return;
    const QSize px = pixelSize();
    m_renderer->resize(px.width(), px.height());
}

void RenderWidget::paintEvent(QPaintEvent*)
{
    if (m_ready)
        m_renderer->render();
}
