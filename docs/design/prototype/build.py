import re
r=lambda f: open(f).read()
js="\n".join(r(f) for f in ['data.js','logic.js','views_a.js','views_b.js','views_c.js'])
css=r('style.css')
html='''<title>de Prototype</title>
<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Schibsted+Grotesk:wght@400;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap">
<style>
'''+css+'''
</style>
<div id="root"></div>
<script>
(function(){
'''+js+'''
})();
</script>
'''
open('index.html','w').write(html)
print(len(html),'bytes',html.count('\n'),'lines')
